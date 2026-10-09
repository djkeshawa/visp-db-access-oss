//! Relation normalization and segment-based blocked-table matching.
use crate::Dialect;
use sqlparser::ast::{Ident, ObjectName};

pub(crate) fn normalize(name: &ObjectName) -> String {
    name.0
        .iter()
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|part| {
            part.as_ident().map_or_else(
                || part.to_string().to_lowercase(),
                |i| i.value.to_lowercase(),
            )
        })
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn cte_key(ident: &Ident, dialect: Dialect) -> String {
    if dialect == Dialect::Postgres && ident.quote_style.is_some() {
        ident.value.clone()
    } else {
        ident.value.to_lowercase()
    }
}

pub(crate) fn matches(pattern: &str, relation: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let (schema, table) = relation.rsplit_once('.').unwrap_or(("", relation));
    match pattern.rsplit_once('.') {
        Some((schema_pattern, table_pattern)) => {
            glob(schema_pattern, schema) && glob(table_pattern, table)
        }
        None => glob(&pattern, table),
    }
}

// A greedy star match uses constant auxiliary space, without regex compilation.
fn glob(pattern: &str, value: &str) -> bool {
    let (p, v) = (pattern.as_bytes(), value.as_bytes());
    let (mut pi, mut vi, mut star, mut retry) = (0, 0, None, 0);
    while vi < v.len() {
        // A pattern star is always a wildcard, even when the value holds a literal '*'.
        if p.get(pi) == Some(&b'*') {
            star = Some(pi);
            pi += 1;
            retry = vi;
        } else if p.get(pi) == v.get(vi) {
            pi += 1;
            vi += 1;
        } else if let Some(pos) = star {
            retry += 1;
            vi = retry;
            pi = pos + 1;
        } else {
            return false;
        }
    }
    while p.get(pi) == Some(&b'*') {
        pi += 1;
    }
    pi == p.len()
}

pub(crate) fn sensitive(relation: &str) -> bool {
    matches!(
        relation,
        "pg_authid"
            | "pg_shadow"
            | "pg_user_mapping"
            | "pg_catalog.pg_authid"
            | "pg_catalog.pg_shadow"
            | "pg_catalog.pg_user_mapping"
            | "mysql.user"
            | "mysql.global_priv"
    )
}

// Some relation-bearing fields in sqlparser 0.63 lack visit_relation annotations.
// Collect these explicitly in the statement callback, while still walking all
// nested queries and expressions with the visitor.
pub(crate) fn supplemental(statement: &sqlparser::ast::Statement) -> Vec<ObjectName> {
    use sqlparser::ast::{
        AlterTableOperation, CommentObject, CreateTableLikeKind, ObjectType, RenameTableNameKind,
        Statement,
    };
    let mut relations = vec![];
    match statement {
        Statement::Drop {
            object_type,
            names,
            table,
            ..
        } => {
            if matches!(
                object_type,
                ObjectType::Table
                    | ObjectType::View
                    | ObjectType::MaterializedView
                    | ObjectType::Sequence
            ) {
                relations.extend(names.iter().cloned());
            }
            relations.extend(table.iter().cloned());
        }
        Statement::RenameTable(renames) => {
            for rename in renames {
                relations.extend([rename.old_name.clone(), rename.new_name.clone()]);
            }
        }
        Statement::Comment {
            object_type,
            object_name,
            ..
        } => {
            if matches!(
                object_type,
                CommentObject::Table | CommentObject::View | CommentObject::MaterializedView
            ) {
                relations.push(object_name.clone());
            } else if *object_type == CommentObject::Column && object_name.0.len() > 1 {
                relations.push(ObjectName(
                    object_name
                        .0
                        .iter()
                        .take(object_name.0.len() - 1)
                        .cloned()
                        .collect(),
                ));
            }
        }
        Statement::CreateTable(table) => {
            relations.extend(table.inherits.iter().flatten().cloned());
            if let Some(
                CreateTableLikeKind::Plain(like) | CreateTableLikeKind::Parenthesized(like),
            ) = &table.like
            {
                relations.push(like.name.clone());
            }
            for constraint in &table.constraints {
                constraint_relation(constraint, &mut relations);
            }
            for column in &table.columns {
                column_relations(column, &mut relations);
            }
        }
        Statement::AlterTable(table) => {
            for operation in &table.operations {
                match operation {
                    AlterTableOperation::AddConstraint { constraint, .. } => {
                        constraint_relation(constraint, &mut relations)
                    }
                    AlterTableOperation::AddColumn { column_def, .. } => {
                        column_relations(column_def, &mut relations)
                    }
                    AlterTableOperation::RenameTable {
                        table_name: RenameTableNameKind::As(name) | RenameTableNameKind::To(name),
                    } => relations.push(name.clone()),
                    AlterTableOperation::ChangeColumn { options, .. }
                    | AlterTableOperation::ModifyColumn { options, .. } => {
                        for option in options {
                            option_relation(option, &mut relations);
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    relations
}

fn constraint_relation(
    constraint: &sqlparser::ast::TableConstraint,
    relations: &mut Vec<ObjectName>,
) {
    if let sqlparser::ast::TableConstraint::ForeignKey(key) = constraint {
        relations.push(key.foreign_table.clone());
    }
}

fn column_relations(column: &sqlparser::ast::ColumnDef, relations: &mut Vec<ObjectName>) {
    for option in &column.options {
        option_relation(&option.option, relations);
    }
}

fn option_relation(option: &sqlparser::ast::ColumnOption, relations: &mut Vec<ObjectName>) {
    if let sqlparser::ast::ColumnOption::ForeignKey(key) = option {
        relations.push(key.foreign_table.clone());
    }
}
