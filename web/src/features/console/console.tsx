import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { format } from 'sql-formatter';
import {
  Play,
  Square,
  PanelLeft,
  WandSparkles,
  PanelRight,
} from 'lucide-react';
import { useSearchParams } from 'react-router-dom';
import type {
  Analysis,
  Cluster,
  Fix,
  QueryResult,
  SchemaTree,
  Approval,
  Policy,
} from '../../api/types';
import { request, json } from '../../api/client';
import { useAction, useResource } from '../../lib/query';
import {
  Button,
  ErrorPanel,
  Field,
  Confirm,
  Modal,
  Picker,
  Skeleton,
  Tip,
  Empty,
  EnvBadge,
} from '../../components/ui';
import { useToast } from '../../components/ui/toast';
import { message } from '../../lib/utils';
import { ClusterHeading } from '../../components/ui/cluster-heading';
import { SqlEditor } from './editor';
import { SchemaExplorer } from './schema-tree';
import { SafetyPanel } from './safety';
import { Results } from './results';
import { QueryTabs } from './query-tabs';
import { QueryLibrary } from './query-library';
import { SplitHandle } from '../../components/ui/split-handle';
import {
  closeTab,
  loadDrafts,
  moveTab,
  type QueryTab,
} from '../../lib/workspace';
import { useUser } from '../auth/session';
import { withClientFixes } from '../../lib/optimize';
const newTab = (index: number, engine = 'postgres'): QueryTab => {
  const sql = `SELECT id, name, email\nFROM ${engine === 'postgres' ? 'public.users' : 'users'}\nORDER BY created_at DESC;`;
  return {
    id: crypto.randomUUID(),
    name: `Query ${index}`,
    sql,
    savedSql: sql,
  };
};
const importedTab = (index: number, engine: string, sql: string): QueryTab => ({
  ...newTab(index, engine),
  sql,
  savedSql: sql,
});
function loadTabs(key: string, engine: string): QueryTab[] {
  try {
    const tabs = loadDrafts(localStorage.getItem(key));
    return tabs.length ? tabs : [newTab(1, engine)];
  } catch {
    return [newTab(1, engine)];
  }
}
export function ConsolePage() {
  const [params, setParams] = useSearchParams();
  const query = useResource<{ items: Cluster[] }>('/clusters');
  const cluster = query.data?.items.find(
    (c) => c.id === params.get('cluster_id'),
  );
  return (
    <>
      <div className="console-page-heading">
        {cluster ? (
          <ClusterHeading cluster={cluster} />
        ) : (
          <div>
            <h1>Query console</h1>
            <p>Explore your data with a safety check on every statement.</p>
          </div>
        )}
        <Picker
          value={cluster?.id ?? ''}
          onChange={(id) => setParams({ cluster_id: id })}
          options={
            query.data?.items.map((c) => ({
              value: c.id,
              label: `${c.name} · ${c.environment}`,
            })) ?? []
          }
          label="Choose cluster"
        />
      </div>
      {query.isPending ? (
        <Skeleton />
      ) : query.error ? (
        <ErrorPanel error={query.error} />
      ) : cluster ? (
        <ConsoleWorkspace
          key={cluster.id}
          cluster={cluster}
          initialSql={params.get('sql') ?? undefined}
        />
      ) : (
        <Empty
          title="Choose a database to start"
          description="Select a cluster above to open its schema and guarded SQL editor."
        />
      )}
    </>
  );
}
export function ConsoleWorkspace({
  cluster,
  initialSql,
}: {
  cluster: Cluster;
  initialSql?: string;
}) {
  const center = useRef<HTMLDivElement>(null);
  const [editorMaximum, setEditorMaximum] = useState(640);
  const user = useUser();
  const policy = useResource<Policy>(`/clusters/${cluster.id}/policy`);
  const storageKey = `vda.tabs.${user?.id ?? 'anonymous'}.${cluster.id}`;
  const [boot] = useState(() => {
    const loaded = loadTabs(storageKey, cluster.engine);
    let activeId = '';
    try {
      activeId = localStorage.getItem(`${storageKey}.active`) ?? '';
    } catch {
      // Storage can be unavailable; fall back to the first tab.
    }
    if (initialSql) {
      // Reuse an identical draft instead of appending a duplicate on remount.
      const tab =
        loaded.find((item) => item.sql === initialSql) ??
        importedTab(loaded.length + 1, cluster.engine, initialSql);
      if (!loaded.includes(tab)) loaded.push(tab);
      activeId = tab.id;
    }
    return { tabs: loaded, active: activeId };
  });
  const [tabs, setTabs] = useState(boot.tabs),
    [active, setActive] = useState(boot.active),
    [rename, setRename] = useState(false),
    [closeConfirm, setCloseConfirm] = useState<'one' | 'others' | null>(null),
    [editorHeight, setEditorHeight] = useState(() =>
      Math.round((innerHeight - 160) * 0.4),
    ),
    [schemaOpen, setSchemaOpen] = useState(
      () => matchMedia('(min-width: 1100px)').matches,
    ),
    [safetyOpen, setSafetyOpen] = useState(
      () => matchMedia('(min-width: 1100px)').matches,
    ),
    [reasonOpen, setReasonOpen] = useState(false),
    [reason, setReason] = useState('');
  useLayoutEffect(() => {
    const element = center.current;
    if (!element) return;
    const measure = () => {
      const resultSpace = matchMedia('(max-width: 600px)').matches ? 340 : 280;
      const maximum = Math.max(140, element.clientHeight - resultSpace - 56);
      setEditorMaximum(maximum);
      setEditorHeight((value) => Math.max(140, Math.min(value, maximum)));
    };
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    measure();
    return () => observer.disconnect();
  }, []);
  const found = tabs.findIndex((tab) => tab.id === active);
  const index = found >= 0 ? found : 0;
  const current = tabs[index] ?? newTab(1, cluster.engine);
  const sql = current.sql;
  const currentId = useRef(current.id);
  useLayoutEffect(() => {
    currentId.current = current.id;
  }, [current.id]);
  const importedSql = useRef(initialSql);
  useEffect(() => {
    if (initialSql && importedSql.current !== initialSql) {
      importedSql.current = initialSql;
      const tab = importedTab(tabs.length + 1, cluster.engine, initialSql);
      setTabs((tabs) => [...tabs, tab]);
      setActive(tab.id);
      setResult(undefined);
    }
  }, [initialSql, tabs.length, cluster.engine]);
  const [debounced, setDebounced] = useState(sql),
    [result, setResult] = useState<QueryResult | undefined>(),
    [executionError, setExecutionError] = useState<unknown>();
  const toast = useToast(),
    cache = useQueryClient(),
    queryId = useRef<string | undefined>(undefined);
  const cancel = useAction('Cancellation requested'),
    approval = useAction<Approval>('Approval requested');
  const schema = useResource<SchemaTree>(`/clusters/${cluster.id}/schema`);
  // Leading/trailing whitespace never changes the verdict, so it neither
  // restarts the debounce nor triggers another round trip.
  const analyzed = debounced.trim();
  useEffect(() => {
    if (sql.trim() === analyzed) return;
    const timer = setTimeout(() => setDebounced(sql), 250);
    return () => clearTimeout(timer);
  }, [sql, analyzed]);
  // Persisting every keystroke serializes all tabs; batch writes instead and
  // flush when the page is hidden or the console unmounts.
  const persist = useRef<() => void>(() => undefined);
  useEffect(() => {
    persist.current = () => {
      try {
        localStorage.setItem(storageKey, JSON.stringify(tabs));
        localStorage.setItem(`${storageKey}.active`, current.id);
      } catch {
        toast('Drafts could not be saved in this browser.', 'error');
      }
      persist.current = () => undefined;
    };
    const timer = setTimeout(() => persist.current(), 500);
    return () => clearTimeout(timer);
  }, [tabs, storageKey, toast, current.id]);
  useEffect(() => {
    const flush = () => persist.current();
    window.addEventListener('pagehide', flush);
    return () => {
      window.removeEventListener('pagehide', flush);
      flush();
    };
  }, []);
  const analysis = useQuery({
    queryKey: ['analysis', cluster.id, analyzed],
    queryFn: ({ signal }) =>
      request<Analysis>(`/clusters/${cluster.id}/analyze`, {
        ...json('POST', { sql: analyzed }),
        signal,
      }),
    enabled: !!analyzed,
    // Revisiting recent text (undo, tab switch) reuses its analysis; the
    // server re-analyzes on every run, so a briefly stale panel is safe.
    staleTime: 15_000,
    // Keep the previous verdict visible while the next one loads, but never
    // across clusters.
    placeholderData: (previous, query) =>
      query?.queryKey[1] === cluster.id ? previous : undefined,
  });
  const running = useMutation({
    mutationFn: ({ sql, id }: { sql: string; id: string; tabId: string }) =>
      request<QueryResult>(
        `/clusters/${cluster.id}/query`,
        json('POST', { sql, query_id: id }),
      ),
    onSuccess: (value, variables) => {
      setResult(value);
      setTabs((tabs) =>
        tabs.map((tab) =>
          tab.id === variables.tabId
            ? { ...tab, savedSql: variables.sql }
            : tab,
        ),
      );
    },
    onError: (error) => setExecutionError(error),
    onSettled: () => {
      queryId.current = undefined;
      void cache.invalidateQueries({ queryKey: ['api'] });
    },
  });
  const analysisFresh =
    sql.trim() === analyzed &&
    !analysis.isFetching &&
    !analysis.isPlaceholderData &&
    !!analysis.data &&
    !analysis.error;
  const verdict = analysis.data?.verdict;
  const needsApproval = verdict === 'requires_approval';
  const canRun =
    !running.isPending &&
    analysisFresh &&
    verdict !== 'deny' &&
    !!sql.trim() &&
    !(needsApproval && cluster.my_access === 'read');
  const run = () => {
    if (!canRun || queryId.current) return;
    if (needsApproval) {
      setReasonOpen(true);
      return;
    }
    const id = crypto.randomUUID();
    queryId.current = id;
    setExecutionError(undefined);
    setResult(undefined);
    running.mutate({ sql, id, tabId: current.id });
  };
  const runRef = useRef(run),
    cancelRef = useRef(() => {
      if (queryId.current)
        cancel.mutate({ path: `/queries/${queryId.current}/cancel` });
    });
  useEffect(() => {
    runRef.current = run;
  });
  useEffect(() => {
    const listener = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
        event.preventDefault();
        runRef.current();
      }
      if (event.key === 'Escape' && queryId.current) {
        event.preventDefault();
        cancelRef.current();
      }
    };
    window.addEventListener('keydown', listener);
    return () => {
      window.removeEventListener('keydown', listener);
      if (queryId.current)
        void request(`/queries/${queryId.current}/cancel`, json('POST')).catch(
          () => undefined,
        );
    };
  }, []);
  // Stable and id-based so memoized children keep their props and an editor
  // echo can never write into a different tab than the one it belongs to.
  const updateSql = useCallback(
    (value: string) =>
      setTabs((tabs) =>
        tabs.some((tab) => tab.id === currentId.current && tab.sql !== value)
          ? tabs.map((tab) =>
              tab.id === currentId.current ? { ...tab, sql: value } : tab,
            )
          : tabs,
      ),
    [],
  );
  const insertSql = useCallback(
    (name: string) =>
      setTabs((tabs) =>
        tabs.map((tab) =>
          tab.id === currentId.current
            ? {
                ...tab,
                sql: `${tab.sql}${tab.sql.endsWith(' ') ? '' : ' '}${name}`,
              }
            : tab,
        ),
      ),
    [],
  );
  const addTab = () => {
    if (running.isPending) return;
    const tab = newTab(tabs.length + 1, cluster.engine);
    setTabs((tabs) => [...tabs, tab]);
    setActive(tab.id);
    setResult(undefined);
    setExecutionError(undefined);
  };
  const finishClose = (kind: 'one' | 'others') => {
    if (kind === 'others') setTabs([current]);
    else {
      const next = closeTab(tabs, index, current.id);
      if (next.tabs.length) {
        setTabs(next.tabs);
        setActive(next.active);
      } else {
        const tab = newTab(1, cluster.engine);
        setTabs([tab]);
        setActive(tab.id);
      }
    }
    setResult(undefined);
    setExecutionError(undefined);
    setCloseConfirm(null);
  };
  const addTabRef = useRef(addTab);
  useEffect(() => {
    addTabRef.current = addTab;
  });
  useEffect(() => {
    const add = () => addTabRef.current();
    const keyboard = (event: KeyboardEvent) => {
      if (
        (event.metaKey || event.ctrlKey) &&
        event.altKey &&
        event.key.toLowerCase() === 't'
      ) {
        event.preventDefault();
        addTabRef.current();
      }
    };
    const reorder = (event: Event) => {
      const { from, to } = (event as CustomEvent<{ from: number; to: number }>)
        .detail;
      setTabs((tabs) => moveTab(tabs, from, to));
    };
    window.addEventListener('vda:new-tab', add);
    window.addEventListener('keydown', keyboard);
    window.addEventListener('vda:reorder-tab', reorder);
    return () => {
      window.removeEventListener('vda:new-tab', add);
      window.removeEventListener('keydown', keyboard);
      window.removeEventListener('vda:reorder-tab', reorder);
    };
  }, []);
  const suggestions = useMemo(
    () => withClientFixes(analyzed, analysis.data, schema.data, cluster.engine),
    [analyzed, analysis.data, schema.data, cluster.engine],
  );
  const freshRef = useRef(analysisFresh);
  useLayoutEffect(() => {
    freshRef.current = analysisFresh;
  });
  const applyFix = useCallback(
    (fix: Fix) => {
      // A fix is computed from the analyzed text; never overwrite newer edits.
      if (!freshRef.current) return;
      if (fix.action === 'replace') {
        updateSql(fix.sql);
        return;
      }
      const tab = importedTab(tabs.length + 1, cluster.engine, fix.sql);
      setTabs((tabs) => [...tabs, tab]);
      setActive(tab.id);
      setResult(undefined);
    },
    [updateSql, tabs.length, cluster.engine],
  );
  const refetchAnalysis = analysis.refetch;
  const retryAnalysis = useCallback(
    () => void refetchAnalysis(),
    [refetchAnalysis],
  );
  return (
    <div
      style={{ '--editor-height': `${editorHeight}px` } as CSSProperties}
      className="console-workspace"
    >
      <div
        className={`console-layout ${schemaOpen ? '' : 'schema-hidden'} ${safetyOpen ? '' : 'safety-hidden'}`}
      >
        {schemaOpen && (
          <aside className="schema-column-panel" aria-label="Schema panel">
            <SchemaExplorer
              onClose={() => setSchemaOpen(false)}
              clusterId={cluster.id}
              engine={cluster.engine}
              onInsert={insertSql}
              onQuery={updateSql}
            />
          </aside>
        )}
        <div className="console-center" ref={center}>
          <QueryTabs
            tabs={tabs}
            active={current.id}
            busy={running.isPending}
            onSelect={(id) => {
              setActive(id);
              setResult(undefined);
              setExecutionError(undefined);
            }}
            onNew={addTab}
            onClose={() =>
              current.sql !== current.savedSql
                ? setCloseConfirm('one')
                : finishClose('one')
            }
            onRename={() => setRename(true)}
            onMove={(direction) =>
              setTabs((tabs) => moveTab(tabs, index, index + direction))
            }
            onCloseOthers={() =>
              tabs.some(
                (tab) => tab.id !== current.id && tab.sql !== tab.savedSql,
              )
                ? setCloseConfirm('others')
                : finishClose('others')
            }
          />
          <div
            className="console-editor"
            role="tabpanel"
            id="active-query-panel"
            aria-labelledby={`query-tab-${current.id}`}
          >
            <div className="editor-toolbar">
              <Button
                variant="ghost"
                aria-label="Toggle schema panel"
                aria-expanded={schemaOpen}
                onClick={() => setSchemaOpen((value) => !value)}
              >
                <PanelLeft size={16} />
              </Button>
              <Button
                variant="primary"
                className={
                  needsApproval ? undefined : `run-${cluster.environment}`
                }
                disabled={!canRun}
                title={
                  verdict === 'deny'
                    ? analysis.data?.issues.find(
                        (issue) => issue.severity === 'block',
                      )?.message
                    : undefined
                }
                onClick={run}
              >
                <Play size={14} />
                {running.isPending
                  ? 'Running…'
                  : needsApproval
                    ? 'Request approval…'
                    : cluster.environment === 'production'
                      ? 'Run on production'
                      : cluster.environment === 'staging'
                        ? 'Run on staging'
                        : 'Run'}
                {!needsApproval && <kbd>⌘ ↵</kbd>}
              </Button>
              {running.isPending && (
                <Button
                  variant="danger"
                  disabled={cancel.isPending}
                  onClick={() => cancelRef.current()}
                >
                  <Square size={12} />
                  Cancel
                </Button>
              )}
              <Button
                aria-label="Format SQL"
                title="Format SQL"
                disabled={running.isPending}
                onClick={() => {
                  try {
                    updateSql(
                      format(sql, {
                        language:
                          cluster.engine === 'postgres'
                            ? 'postgresql'
                            : 'mysql',
                      }),
                    );
                  } catch (error) {
                    toast(message(error), 'error');
                  }
                }}
              >
                <WandSparkles size={14} />
                <span className="format-label">Format</span>
              </Button>
              <QueryLibrary
                busy={running.isPending}
                clusterId={cluster.id}
                sql={sql}
                onRestore={(value) => {
                  const tab = importedTab(
                    tabs.length + 1,
                    cluster.engine,
                    value,
                  );
                  setTabs((tabs) => [...tabs, tab]);
                  setActive(tab.id);
                  setResult(undefined);
                }}
                onSaved={() =>
                  setTabs((tabs) =>
                    tabs.map((tab) =>
                      tab.id === current.id
                        ? { ...tab, savedSql: tab.sql }
                        : tab,
                    ),
                  )
                }
              />
              <Tip text="Toggle safety panel">
                <Button
                  variant="ghost"
                  className="icon"
                  aria-label="Toggle safety panel"
                  aria-expanded={safetyOpen}
                  onClick={() => setSafetyOpen((value) => !value)}
                >
                  <PanelRight size={16} />
                </Button>
              </Tip>
            </div>
            <SqlEditor
              value={sql}
              onChange={updateSql}
              onRun={() => runRef.current()}
              engine={cluster.engine}
              schema={schema.data}
              readOnly={running.isPending}
            />
            <div className="editor-hint">
              {!safetyOpen && (
                <Button
                  variant="ghost"
                  className={`compact-verdict ${verdict === 'allow' ? 'success-text' : verdict === 'deny' ? 'error-text' : 'warning-text'}`}
                  aria-label="Open safety analysis"
                  onClick={() => setSafetyOpen(true)}
                >
                  {analysis.error
                    ? 'Analysis unavailable'
                    : !analysisFresh
                      ? 'Analyzing…'
                      : verdict === 'allow'
                        ? 'Safe to run'
                        : needsApproval
                          ? 'Needs approval'
                          : 'Blocked'}
                </Button>
              )}

              <span>⌘ / Ctrl + Enter to run</span>
              <span>Esc to cancel</span>
            </div>
          </div>
          <SplitHandle
            value={editorHeight}
            onChange={setEditorHeight}
            minimum={140}
            maximum={editorMaximum}
          />
          <div className="console-results">
            {!!executionError && (
              <>
                <ErrorPanel error={executionError} />
                <p className="query-error-hint">
                  Check table and column names in Schema, then review the
                  cluster policy. Correct the SQL before running again.
                </p>
              </>
            )}
            {result ? (
              <Results key={result.query_id} result={result} />
            ) : (
              <div className="results-placeholder">
                <TerminalPlaceholder running={running.isPending} />
              </div>
            )}
          </div>
        </div>
        {safetyOpen && (
          <aside className="safety-column" aria-label="Safety panel">
            <div className="side-panel-close">
              <Button
                variant="ghost"
                className="icon"
                aria-label="Close safety panel"
                onClick={() => setSafetyOpen(false)}
              >
                <PanelRight size={16} />
              </Button>
            </div>
            <SafetyPanel
              analysis={analysis.data}
              maxRows={policy.data?.max_rows}
              pending={sql.trim() !== analyzed || analysis.isFetching}
              error={analysis.error}
              clusterId={cluster.id}
              retry={retryAnalysis}
              suggestions={suggestions}
              onApplyFix={applyFix}
            />
          </aside>
        )}
      </div>
      <Modal open={rename} onOpenChange={setRename} title="Rename query tab">
        <form
          onSubmit={(event) => {
            event.preventDefault();
            const name = String(
              new FormData(event.currentTarget).get('name'),
            ).trim();
            if (name) {
              setTabs((tabs) =>
                tabs.map((tab) =>
                  tab.id === current.id ? { ...tab, name } : tab,
                ),
              );
              setRename(false);
            }
          }}
        >
          <Field label="Tab name">
            <input
              name="name"
              defaultValue={current.name}
              required
              maxLength={80}
            />
          </Field>
          <div className="dialog-actions">
            <Button type="submit" variant="primary">
              Rename tab
            </Button>
          </div>
        </form>
      </Modal>
      <Confirm
        open={!!closeConfirm}
        onOpenChange={(open) => {
          if (!open) setCloseConfirm(null);
        }}
        title="Discard changed query drafts?"
        description="These tabs have SQL changes since their last run or favorite. Closing removes their browser drafts."
        danger
        onConfirm={() => {
          if (closeConfirm) finishClose(closeConfirm);
        }}
      />
      <Modal
        open={reasonOpen}
        onOpenChange={setReasonOpen}
        title="Request query approval"
        description={`This write on ${cluster.name} requires review by another cluster administrator.`}
      >
        <EnvBadge environment={cluster.environment} />
        <pre
          className="sql-preview-block"
          tabIndex={0}
          aria-label="SQL preview"
        >
          {sql}
        </pre>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            approval.mutate(
              {
                path: '/approvals',
                body: { cluster_id: cluster.id, sql, reason },
              },
              {
                onSuccess: () => {
                  setReasonOpen(false);
                  setReason('');
                },
              },
            );
          }}
        >
          <Field label="Reason template">
            <Picker
              label="Reason template"
              value="custom"
              onChange={(value) => {
                if (value !== 'custom')
                  setReason(
                    value === 'support'
                      ? 'Support correction: [ticket]. Target rows verified with [SELECT]. Expected change: [count] rows.'
                      : 'Scheduled maintenance: [change]. Verified target and rollback plan: [details].',
                  );
              }}
              options={[
                { value: 'custom', label: 'Write a reason' },
                { value: 'support', label: 'Support correction' },
                { value: 'maintenance', label: 'Scheduled maintenance' },
              ]}
            />
          </Field>
          <p className="muted">
            Who can approve: another administrator on this cluster. You cannot
            approve your own request.
          </p>
          <Field
            label="Reason"
            hint="Explain the intended change and how you verified the target rows."
          >
            <textarea
              value={reason}
              onChange={(event) => setReason(event.target.value)}
              required
              rows={3}
            />
          </Field>
          <div className="dialog-actions">
            <Button onClick={() => setReasonOpen(false)}>Cancel</Button>
            <Button
              type="submit"
              variant="primary"
              disabled={approval.isPending || !reason.trim()}
            >
              {approval.isPending ? 'Requesting…' : 'Request approval'}
            </Button>
          </div>
        </form>
      </Modal>
    </div>
  );
}
function TerminalPlaceholder({ running }: { running: boolean }) {
  return (
    <div className="console-empty" role="status">
      <h2>{running ? 'Query in progress…' : 'No results yet'}</h2>
      <p>
        {running
          ? 'The gateway is executing your guarded SQL.'
          : 'Run a query to see its results here.'}
      </p>
    </div>
  );
}
