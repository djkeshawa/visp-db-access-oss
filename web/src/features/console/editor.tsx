import { useEffect, useRef } from 'react';
import { Compartment, EditorState, Prec } from '@codemirror/state';
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  drawSelection,
} from '@codemirror/view';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { autocompletion } from '@codemirror/autocomplete';
import {
  bracketMatching,
  syntaxHighlighting,
  defaultHighlightStyle,
  HighlightStyle,
} from '@codemirror/language';
import { sql, PostgreSQL, MySQL } from '@codemirror/lang-sql';
import type { Engine, SchemaTree } from '../../api/types';
const editorTheme = EditorView.theme({
  '&': {
    fontSize: 'var(--editor-font-size, var(--text-sm))',
    backgroundColor: 'var(--surface)',
    color: 'var(--text)',
  },
  '.cm-content': {
    fontFamily: 'var(--font-mono)',
    padding: 'var(--space-4) 0',
  },
  '.cm-gutters': {
    backgroundColor: 'var(--subtle)',
    color: 'var(--muted)',
    borderRight: 'var(--stroke) solid var(--border)',
  },
  '.cm-activeLine': { backgroundColor: 'var(--subtle)' },
  '.cm-tooltip-autocomplete > ul > li[aria-selected]': {
    backgroundColor: 'var(--selection)',
    color: 'var(--ink)',
  },
  '&.cm-focused .cm-matchingBracket, &.cm-focused .cm-nonmatchingBracket, .cm-selectionMatch':
    {
      backgroundColor: 'var(--selection)',
      outline: '1px solid var(--graphite)',
    },
  '.cm-panels': {
    backgroundColor: 'var(--subtle)',
    color: 'var(--ink)',
  },
  '.cm-searchMatch': {
    backgroundColor: 'var(--selection)',
    outline: '1px solid var(--graphite)',
  },
  '.cm-searchMatch-selected': {
    backgroundColor: 'var(--selection)',
    outline: '2px solid var(--ink)',
  },
  '.cm-button, .cm-textfield': {
    background: 'var(--surface)',
    color: 'var(--ink)',
    border: '1px solid var(--rule)',
    borderRadius: '7px',
  },
  '.cm-activeLineGutter': { backgroundColor: 'var(--subtle)' },
  '&.cm-focused': { outline: 'none' },
  '.cm-cursor': { borderLeftColor: 'var(--text)' },
  '.cm-tooltip': {
    backgroundColor: 'var(--material)',
    border: 'var(--stroke) solid var(--border)',
    color: 'var(--text)',
    boxShadow: 'var(--shadow)',
  },
  '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': {
    backgroundColor: 'var(--selection)',
  },
});
const highlightStyle = syntaxHighlighting(
  HighlightStyle.define(
    defaultHighlightStyle.specs.map((style) => ({
      ...style,
      color: 'var(--ink)',
    })),
  ),
);
const languageExtension = (engine: Engine, schema?: SchemaTree) => {
  const sqlSchema: Record<string, Record<string, string[]>> = {};
  for (const namespace of schema?.schemas ?? []) {
    sqlSchema[namespace.name] = Object.fromEntries(
      namespace.tables.map((table) => [
        table.name,
        table.columns.map((column) => column.name),
      ]),
    );
  }
  return sql({
    dialect: engine === 'mysql' ? MySQL : PostgreSQL,
    schema: sqlSchema,
    defaultSchema: engine === 'postgres' ? 'public' : undefined,
  });
};
const accessExtension = (readOnly: boolean) => [
  EditorState.readOnly.of(readOnly),
  EditorView.editable.of(!readOnly),
  EditorView.contentAttributes.of({
    'aria-label': readOnly ? 'SQL preview' : 'SQL editor',
    spellcheck: 'false',
  }),
  EditorView.theme({
    '.cm-content': {
      minHeight: readOnly
        ? 'var(--preview-height)'
        : 'var(--editor-height, var(--editor-default-height))',
    },
  }),
];
const viewportLabel = (readOnly: boolean) =>
  readOnly ? 'SQL preview viewport' : 'SQL editor viewport';
/** CodeMirror SQL editor with dialect and schema-aware completions. */
export function SqlEditor({
  value,
  onChange,
  onRun,
  engine = 'postgres',
  schema,
  readOnly = false,
}: {
  value: string;
  onChange?: (value: string) => void;
  /** Bound to Mod-Enter; takes precedence over CodeMirror's insertBlankLine. */
  onRun?: () => void;
  engine?: Engine;
  schema?: SchemaTree;
  readOnly?: boolean;
}) {
  const host = useRef<HTMLDivElement>(null),
    view = useRef<EditorView | null>(null),
    change = useRef(onChange),
    runQuery = useRef(onRun),
    initial = useRef(value),
    first = useRef({ engine, schema, readOnly }),
    language = useRef(new Compartment()),
    access = useRef(new Compartment());
  useEffect(() => {
    change.current = onChange;
    runQuery.current = onRun;
  }, [onChange, onRun]);
  // The view is created once so undo history, selection and focus survive run
  // state changes and schema refetches; those reconfigure compartments instead.
  useEffect(() => {
    if (!host.current) return;
    const options = first.current;
    const editor = new EditorView({
      parent: host.current,
      state: EditorState.create({
        doc: initial.current,
        extensions: [
          lineNumbers(),
          history(),
          drawSelection(),
          highlightActiveLine(),
          bracketMatching(),
          highlightStyle,
          language.current.of(
            languageExtension(options.engine, options.schema),
          ),
          autocompletion({ icons: false }),
          Prec.highest(
            keymap.of([
              {
                key: 'Mod-Enter',
                run: () => {
                  if (!runQuery.current) return false;
                  runQuery.current();
                  return true;
                },
              },
            ]),
          ),
          keymap.of([...defaultKeymap, ...historyKeymap]),
          access.current.of(accessExtension(options.readOnly)),
          EditorView.updateListener.of((update) => {
            if (update.docChanged)
              change.current?.(update.state.doc.toString());
          }),
          editorTheme,
        ],
      }),
    });
    editor.scrollDOM.tabIndex = 0;
    editor.scrollDOM.setAttribute(
      'aria-label',
      viewportLabel(options.readOnly),
    );
    view.current = editor;
    return () => {
      editor.destroy();
      view.current = null;
    };
  }, []);
  useEffect(() => {
    view.current?.dispatch({
      effects: language.current.reconfigure(languageExtension(engine, schema)),
    });
  }, [engine, schema]);
  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    editor.dispatch({
      effects: access.current.reconfigure(accessExtension(readOnly)),
    });
    editor.scrollDOM.setAttribute('aria-label', viewportLabel(readOnly));
  }, [readOnly]);
  useEffect(() => {
    initial.current = value;
    const editor = view.current;
    if (editor && value !== editor.state.doc.toString())
      editor.dispatch({
        changes: { from: 0, to: editor.state.doc.length, insert: value },
      });
  }, [value]);
  return <div className="sql-editor" ref={host} />;
}
