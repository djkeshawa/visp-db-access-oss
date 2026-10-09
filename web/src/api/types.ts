// Mirrors docs/API.md. Credentials are write-only.
export type OrgRole = 'admin' | 'member';
export type AccessLevel = 'read' | 'write' | 'admin';
export type Engine = 'postgres' | 'mysql';
export type Provider = 'aws' | 'gcp' | 'azure' | 'onprem' | 'other';
export type Environment = 'production' | 'staging' | 'development';
export type TlsMode = 'disable' | 'prefer' | 'require' | 'verify_full';
export type HealthStatus = 'healthy' | 'degraded' | 'down' | 'unknown';
export type Verdict = 'allow' | 'requires_approval' | 'deny';
export type Severity = 'info' | 'warning' | 'block';
export type Risk = 'low' | 'medium' | 'high' | 'critical';
export type StatementKind =
  | 'select'
  | 'explain'
  | 'show'
  | 'insert'
  | 'update'
  | 'delete'
  | 'merge'
  | 'ddl'
  | 'dcl'
  | 'transaction'
  | 'utility'
  | 'other';

export interface User {
  id: string;
  email: string;
  name: string;
  org_role: OrgRole;
  disabled: boolean;
  created_at: string;
  last_login_at: string | null;
}

export interface Project {
  id: string;
  name: string;
  description: string;
  cluster_count: number;
  created_at: string;
}

export interface Health {
  status: HealthStatus;
  latency_ms: number | null;
  server_version: string | null;
  is_replica: boolean | null;
  active_connections: number | null;
  max_connections: number | null;
  error: string | null;
  checked_at: string | null;
}

export interface Cluster {
  id: string;
  project_id: string;
  name: string;
  engine: Engine;
  provider: Provider;
  region: string;
  environment: Environment;
  host: string;
  port: number;
  database: string;
  username: string;
  tls_mode: TlsMode;
  replica_host: string | null;
  replica_port: number | null;
  tags: Record<string, string>;
  health: Health;
  my_access: AccessLevel | null; // effective level of the caller
  created_at: string;
  updated_at: string;
}
// password is write-only: accepted on create/update, never returned.

export interface Policy {
  max_rows: number; // default 1000 (prod) / 5000 (non-prod)
  statement_timeout_ms: number; // default 15000 (prod) / 60000
  lock_timeout_ms: number; // default 2000
  max_cost: number | null; // EXPLAIN total cost gate for reads
  max_concurrent_queries: number; // per node, default 4
  allow_writes: boolean; // default false in production
  require_approval_for_writes: boolean; // default true
  max_affected_rows: number; // default 1000
  allow_ddl: boolean; // default false
  route_reads_to_replica: boolean; // default true
  masked_columns: string[]; // patterns: "email", "users.ssn", "*.password*"
  blocked_tables: string[]; // patterns: "secrets.*", "audit_trail"
  allowed_cidrs: string[]; // empty = only org-level rule applies
}

export interface Issue {
  severity: Severity;
  code: string;
  message: string;
}
export interface StatementAnalysis {
  kind: StatementKind;
  sql: string;
  tables: string[];
  functions: string[];
  risk: Risk;
  issues: Issue[];
  has_where: boolean;
  has_limit: boolean;
}
export interface Analysis {
  verdict: Verdict;
  risk: Risk;
  statements: StatementAnalysis[];
  rewritten_sql: string | null;
  issues: Issue[];
  /** Advisory performance hints; absent on analyses stored by older servers. */
  suggestions?: Suggestion[];
} // issues = flattened, incl. top-level

export interface Fix {
  label: string;
  sql: string;
  /** `replace` swaps the editor text; `new_tab` opens the SQL alongside it. */
  action: 'replace' | 'new_tab';
}
export interface Suggestion {
  code: string;
  message: string;
  fix?: Fix;
}

export interface Column {
  name: string;
  type_name: string;
  masked: boolean;
}
export interface QueryResult {
  query_id: string;
  columns: Column[];
  rows: unknown[][];
  row_count: number;
  truncated: boolean;
  affected_rows: number | null;
  elapsed_ms: number;
  executed_sql: string;
  routed_to: 'primary' | 'replica';
  analysis: Analysis;
}

export interface HistoryEntry {
  id: string;
  cluster_id: string;
  cluster_name: string;
  user_id: string;
  user_email: string;
  sql: string;
  verdict: Verdict;
  status: 'running' | 'unknown' | 'ok' | 'blocked' | 'error' | 'cancelled';
  row_count: number | null;
  elapsed_ms: number | null;
  error: string | null;
  created_at: string;
}

export interface Approval {
  id: string;
  cluster_id: string;
  cluster_name: string;
  requester: { id: string; email: string; name: string };
  sql: string;
  reason: string;
  analysis: Analysis;
  status:
    | 'pending'
    | 'approved'
    | 'rejected'
    | 'executing'
    | 'executed'
    | 'failed'
    | 'expired';
  error: string | null;
  sql_truncated: boolean;
  reviewer: { id: string; email: string; name: string } | null;
  review_note: string | null;
  result: QueryResult | null;
  created_at: string;
  reviewed_at: string | null;
  executed_at: string | null;
  expires_at: string;
}

export interface Grant {
  id: string;
  user: { id: string; email: string; name: string };
  scope: 'project' | 'cluster';
  scope_id: string;
  scope_name: string;
  level: AccessLevel;
  expires_at: string | null;
  created_at: string;
  created_by: string | null;
}

export interface AuditEvent {
  id: string;
  actor: { id: string; email: string } | null;
  action: string;
  target_type: string | null;
  target_id: string | null;
  ip: string | null;
  details: unknown;
  created_at: string;
}

export interface SchemaTree {
  schemas: {
    name: string;
    tables: {
      name: string;
      kind: 'table' | 'view' | 'materialized_view';
      row_estimate: number | null;
      columns: {
        name: string;
        data_type: string;
        nullable: boolean;
        is_primary_key: boolean;
      }[];
    }[];
  }[];
}

export interface Page<T> {
  items: T[];
  next_cursor: string | null;
}

export type UserLookup = Pick<User, 'id' | 'email' | 'name'>;

export type DiscoveryProvider = 'aws';
export type ResourceStatus = 'new' | 'imported' | 'ignored' | 'gone';
export type Drift =
  'endpoint_changed' | 'replica_changed' | 'deleted' | 'engine_changed';
export interface ScanRun {
  id: string;
  source_id: string;
  status: 'running' | 'succeeded' | 'partial' | 'failed';
  started_at: string;
  finished_at: string | null;
  found: number;
  new: number;
  gone: number;
  changed: number;
  errors: { region: string | null; message: string }[];
}
export interface DiscoverySource {
  id: string;
  provider: DiscoveryProvider;
  name: string;
  role_arn: string | null;
  external_id_set: boolean;
  regions: string[];
  default_project_id: string | null;
  environment_tag_keys: string[];
  scan_interval_minutes: number;
  enabled: boolean;
  last_scan: ScanRun | null;
  last_test:
    | (Pick<DiscoveryTest, 'ok' | 'account_id' | 'identity_arn'> & {
        tested_at: string;
      })
    | null;
  created_at: string;
  updated_at: string;
}
export interface DiscoveredResource {
  id: string;
  source_id: string;
  source_name: string;
  provider: DiscoveryProvider;
  kind: 'rds_instance' | 'aurora_cluster';
  arn: string;
  identifier: string;
  account_id: string;
  region: string;
  engine: Engine;
  engine_detail: string;
  engine_version: string;
  host: string;
  port: number;
  replica_host: string | null;
  replica_port: number | null;
  database: string | null;
  status_detail: string;
  publicly_accessible: boolean;
  encrypted: boolean;
  multi_az: boolean;
  iam_auth_enabled: boolean;
  vpc_id: string | null;
  tags: Record<string, string>;
  suggested_environment: Environment;
  status: ResourceStatus;
  cluster_id: string | null;
  drift: Drift[];
  first_seen_at: string;
  last_seen_at: string;
}
export interface DiscoveryTest {
  ok: boolean;
  account_id: string | null;
  identity_arn: string | null;
  regions: { region: string; ok: boolean; error: string | null }[];
}
export type DiscoveryCounts = Record<ResourceStatus, number>;
export interface ResourcePage extends Page<DiscoveredResource> {
  counts: DiscoveryCounts;
}
export interface SourceDraft {
  provider: DiscoveryProvider;
  name: string;
  role_arn: string | null;
  external_id?: string | null;
  regions: string[];
  default_project_id: string | null;
  environment_tag_keys: string[];
  scan_interval_minutes: number;
  enabled: boolean;
}
