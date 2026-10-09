import { memo } from 'react';
import {
  ShieldCheck,
  ShieldAlert,
  ShieldX,
  Info,
  AlertTriangle,
  Lightbulb,
} from 'lucide-react';
import type { Analysis, Fix, Suggestion } from '../../api/types';
import { Link } from 'react-router-dom';
import { Button, ErrorPanel } from '../../components/ui';
export const SafetyPanel = memo(function SafetyPanel({
  analysis,
  pending = false,
  error,
  maxRows,
  clusterId,
  retry,
  suggestions,
  onApplyFix,
}: {
  analysis?: Analysis;
  pending?: boolean;
  error?: unknown;
  maxRows?: number;
  clusterId?: string;
  retry?: () => void;
  /** Defaults to the analysis' own suggestions. */
  suggestions?: Suggestion[];
  /** Enables fix buttons; omitted where SQL cannot be edited (e.g. approvals). */
  onApplyFix?: (fix: Fix) => void;
}) {
  const verdict = analysis?.verdict;
  const hints =
    verdict === 'deny' ? [] : (suggestions ?? analysis?.suggestions ?? []);
  // Keep the last verdict on screen while re-analyzing so typing never flickers.
  const analyzing = pending && !analysis;
  const Icon =
    verdict === 'allow'
      ? ShieldCheck
      : verdict === 'requires_approval'
        ? ShieldAlert
        : ShieldX;
  const tone =
    verdict === 'allow'
      ? 'success'
      : verdict === 'requires_approval'
        ? 'warning'
        : 'danger';
  return (
    <section className="safety-panel" aria-label="SQL safety analysis">
      <div className="section-toolbar">
        <strong>Safety check</strong>
        {clusterId ? (
          <Link to={`/clusters/${clusterId}?tab=policy`}>View policy</Link>
        ) : (
          <span className="muted">Live analysis</span>
        )}
      </div>
      {error ? (
        <ErrorPanel error={error} retry={retry} />
      ) : (
        <>
          <div
            className={`verdict ${analyzing ? 'neutral' : tone}`}
            aria-live="polite"
            aria-busy={pending}
          >
            <Icon size={21} />
            <strong>
              {analyzing
                ? 'Analyzing…'
                : verdict === 'allow'
                  ? 'Safe to run'
                  : verdict === 'requires_approval'
                    ? 'Needs approval'
                    : analysis
                      ? 'Blocked'
                      : 'Waiting for SQL'}
            </strong>
            {pending && analysis && (
              <small className="verdict-updating">Updating…</small>
            )}
          </div>
          {analysis && (
            <>
              <div className="risk-row">
                <span>Risk</span>
                <span>{analysis.risk}</span>
              </div>
              <div className={`risk-meter ${analysis.risk}`} aria-hidden="true">
                <i />
                <i />
                <i />
                <i />
              </div>
              <div
                className="risk-labels"
                aria-label={`Risk level: ${analysis.risk}`}
              >
                {['low', 'medium', 'high', 'critical'].map((risk) => (
                  <span
                    key={risk}
                    className={risk === analysis.risk ? 'active' : undefined}
                  >
                    {risk.charAt(0).toUpperCase() + risk.slice(1)}
                  </span>
                ))}
              </div>
              <div className="issues">
                {analysis.issues.map((issue, i) => (
                  <div
                    className={`issue ${issue.severity}`}
                    key={`${issue.code}-${i}`}
                  >
                    {issue.severity === 'info' ? (
                      <Info size={15} />
                    ) : (
                      <AlertTriangle size={15} />
                    )}
                    <span>
                      {issue.message}
                      {issue.code === 'masked_data_serialization' && (
                        <small>
                          {' '}
                          Select individual columns so the gateway can mask
                          protected values.
                        </small>
                      )}
                      <details className="issue-help">
                        <summary>Why and how to fix</summary>
                        <p>{issueHelp(issue.code)}</p>
                      </details>
                    </span>
                  </div>
                ))}
              </div>
              {hints.length > 0 && (
                <div
                  className="suggestions"
                  aria-label="Optimization suggestions"
                >
                  <strong>Optimizations</strong>
                  {hints.map((hint) => (
                    <div className="suggestion" key={hint.code}>
                      <Lightbulb size={15} aria-hidden="true" />
                      <span>
                        {hint.message}
                        {hint.fix && onApplyFix && (
                          <Button
                            size="small"
                            disabled={pending}
                            title={
                              hint.fix.action === 'new_tab'
                                ? 'Opens in a new tab'
                                : 'Replaces the editor text (undo with Ctrl/Cmd+Z)'
                            }
                            onClick={() => hint.fix && onApplyFix(hint.fix)}
                          >
                            {hint.fix.label}
                          </Button>
                        )}
                      </span>
                    </div>
                  ))}
                </div>
              )}
              <dl className="analysis-facts">
                <dt>Statement</dt>
                <dd>
                  {analysis.statements
                    .map((statement) => statement.kind)
                    .join(', ')}
                </dd>
                <dt>Tables</dt>
                <dd>
                  {Array.from(
                    new Set(
                      analysis.statements.flatMap(
                        (statement) => statement.tables,
                      ),
                    ),
                  ).join(', ') || 'None'}
                </dd>
              </dl>
              {analysis.rewritten_sql && (
                <details open className="rewritten">
                  <summary>What will actually run</summary>
                  {maxRows !== undefined &&
                    new RegExp(
                      `(?:\\bLIMIT\\s+(?:\\d+\\s*,\\s*)?${maxRows + 1}(?:\\s+OFFSET\\s+\\d+)?|\\bFETCH\\s+(?:FIRST|NEXT)\\s+${maxRows + 1}\\s+ROWS?\\s+ONLY)\\s*;?\\s*$`,
                      'i',
                    ).test(analysis.rewritten_sql) && (
                      <small className="muted">
                        (+1 row to detect truncation)
                      </small>
                    )}
                  <pre>
                    {analysis.rewritten_sql
                      .split(/(LIMIT\s+\d+)/gi)
                      .map((text, i) =>
                        /^LIMIT\s+\d+$/i.test(text) ? (
                          <mark key={i}>{text}</mark>
                        ) : (
                          text
                        ),
                      )}
                  </pre>
                </details>
              )}
            </>
          )}
        </>
      )}
    </section>
  );
});

function issueHelp(code: string): string {
  const hints: Record<string, string> = {
    missing_where:
      'A write without a WHERE condition can affect the whole table. Add a precise WHERE condition and verify the target rows with SELECT.',
    blocked_table:
      'The cluster policy protects this table. Query an allowed table, or ask a policy owner to review the restriction.',
    blocked_function:
      'This function can bypass query protections or change server state. Remove it and use an allowed SQL expression.',
    write_approval:
      'Writes require a second person to review the SQL. Include the expected row count and verification in your approval reason.',
    limit_added:
      'The policy bounds the result size to protect the database. Narrow your WHERE condition if you need a different set of rows.',
    ddl_disabled:
      'Schema changes are disabled by this cluster policy. Ask the policy owner to review the intended change.',
    masked_data_serialization:
      'Whole-row serialization prevents reliable column masking. Select individual columns so the gateway can mask protected values.',
  };
  return (
    hints[code] ??
    'The gateway checks SQL against your access and the cluster policy. Review the statement and policy, then correct the SQL or ask a policy owner for help.'
  );
}
