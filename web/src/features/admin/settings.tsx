import { useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { useUser } from '../auth/session';
import { AboutConsole } from './about';
import { PersonalSettings } from './preferences';
import { DiscoverySources } from '../discovery/sources';
import { useAction, useResource } from '../../lib/query';
import {
  Button,
  ChipInput,
  Confirm,
  ErrorPanel,
  Skeleton,
  TabBar,
  SwitchRow,
} from '../../components/ui';
import { isCidr } from '../../lib/utils';
type Network = { allowed_cidrs: string[]; trust_proxy_headers: boolean };
export function SettingsPage() {
  const [params, setParams] = useSearchParams();
  const user = useUser(),
    admin = user?.org_role === 'admin';
  const tab =
    admin && params.get('tab') !== 'preferences'
      ? params.get('tab') === 'discovery'
        ? 'discovery'
        : 'general'
      : 'preferences';
  return (
    <TabBar
      value={tab}
      onChange={(value) => setParams({ tab: value })}
      items={[
        ...(admin
          ? [
              { value: 'general', label: 'General' },
              { value: 'discovery', label: 'Cloud discovery' },
            ]
          : []),
        { value: 'preferences', label: 'Profile and preferences' },
      ]}
    >
      {tab === 'preferences' ? (
        <PersonalSettings />
      ) : tab === 'discovery' ? (
        <DiscoverySources />
      ) : (
        <GeneralSettings />
      )}
    </TabBar>
  );
}
function GeneralSettings() {
  const query = useResource<Network>('/settings/network');
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Workspace settings</h1>
          <p>
            Organization-wide network protection and application information.
          </p>
        </div>
      </div>
      {query.isPending ? (
        <Skeleton />
      ) : query.error ? (
        <ErrorPanel error={query.error} />
      ) : (
        <NetworkForm key={JSON.stringify(query.data)} original={query.data} />
      )}
      <AboutConsole />
    </>
  );
}
function NetworkForm({ original }: { original: Network }) {
  const [draft, setDraft] = useState(original),
    [confirm, setConfirm] = useState(false);
  const save = useAction('Network policy saved');
  const invalid = draft.allowed_cidrs.some((cidr) => !isCidr(cidr)),
    changed = JSON.stringify(original) !== JSON.stringify(draft);
  return (
    <div className="settings-page">
      <section className="settings-section">
        <h2>Network allowlist</h2>
        <p className="muted">
          Restrict the client networks that can reach database queries.
        </p>
        <ChipInput
          label="Organization allowed CIDRs"
          value={draft.allowed_cidrs}
          onChange={(allowed_cidrs) =>
            setDraft((draft) => ({ ...draft, allowed_cidrs }))
          }
          validate={isCidr}
          help="Use CIDR notation, such as 10.0.0.0/8 or 2001:db8::/32."
        />
        <SwitchRow
          label="Trust proxy headers"
          description="Use forwarded client IPs only behind a trusted reverse proxy."
          checked={draft.trust_proxy_headers}
          onChange={(trust_proxy_headers) =>
            setDraft((draft) => ({ ...draft, trust_proxy_headers }))
          }
        />
        <div className="callout warning">
          Changing network rules can lock out colleagues. Only trust proxy
          headers behind a trusted reverse proxy. The server refuses rules that
          would lock out your current IP.
        </div>
        {invalid && (
          <p className="error-text" role="alert">
            Remove or correct the highlighted CIDRs before saving.
          </p>
        )}
        <Button
          variant="primary"
          disabled={!changed || invalid}
          onClick={() => setConfirm(true)}
        >
          Review network changes
        </Button>
      </section>
      <Confirm
        open={confirm}
        onOpenChange={setConfirm}
        title="Update organization network policy?"
        description="These rules affect everyone. Confirm that your trusted networks and reverse proxy settings are correct."
        busy={save.isPending}
        onConfirm={() =>
          save.mutate(
            { path: '/settings/network', method: 'PUT', body: draft },
            { onSuccess: () => setConfirm(false) },
          )
        }
      >
        <pre className="json-preview">{JSON.stringify(draft, null, 2)}</pre>
      </Confirm>
    </div>
  );
}
