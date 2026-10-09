import { test, expect } from '@playwright/test';
test.beforeEach(async ({ page }) => {
  await page.goto('/login');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: 'Workspace overview' }),
  ).toBeVisible();
});
test('production console runs, masks, exports, blocks and requests approval', async ({
  page,
}) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.getByRole('link', { name: 'Clusters', exact: true }).click();
  await expect(
    page.getByRole('table', { name: 'Production clusters' }),
  ).toBeVisible();
  await page
    .getByRole('link')
    .filter({ hasText: 'commerce-primary' })
    .first()
    .click();
  await expect(page.getByText('Safe to run')).toBeVisible();
  await expect(page.locator('main')).toHaveAttribute(
    'data-environment',
    'production',
  );
  await page.getByRole('button', { name: 'Run', exact: false }).click();
  await expect(page.getByRole('grid', { name: 'Query results' })).toBeVisible();
  await expect(page.getByText('Routed to')).toContainText('replica');
  await expect(page.getByRole('columnheader', { name: 'email' })).toBeVisible();
  await expect(
    page.getByRole('gridcell').filter({ hasText: 'Maya Patel' }).first(),
  ).toBeVisible();
  await page
    .locator('.cell-value')
    .filter({ hasText: '"plan"' })
    .first()
    .click();
  await expect(page.getByRole('dialog', { name: 'JSON value' })).toBeVisible();
  await page
    .getByRole('dialog', { name: 'JSON value' })
    .getByRole('button', { name: 'Close dialog' })
    .click();
  const resize = page.getByRole('separator', { name: 'Resize name' });
  const initialWidth = Number(await resize.getAttribute('aria-valuenow'));
  await resize.focus();
  await page.keyboard.press('ArrowRight');
  await expect(resize).toHaveAttribute(
    'aria-valuenow',
    String(initialWidth + 20),
  );
  await page.screenshot({
    path: 'screenshots/console-results.png',
    fullPage: true,
    animations: 'disabled',
  });
  const download = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Export', exact: true }).click();
  await page
    .getByRole('menuitem', { name: 'Download CSV', exact: true })
    .click();
  expect((await download).suggestedFilename()).toBe('query-result.csv');
  const editor = page.getByRole('textbox', { name: 'SQL editor' });
  await editor.fill('DELETE FROM users');
  await expect(page.getByText('Blocked', { exact: true })).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Run', exact: false }),
  ).toBeDisabled();
  await editor.fill("UPDATE users SET name = 'Maya' WHERE id = 42");
  await expect(page.getByText('Needs approval')).toBeVisible();
  await page
    .getByRole('button', { name: 'Request approval…', exact: true })
    .click();
  await page
    .getByLabel('Reason', { exact: true })
    .fill('Verified support correction');
  await page
    .getByRole('button', { name: 'Request approval', exact: true })
    .click();
  await expect(
    page.getByText('Approval requested', { exact: true }),
  ).toBeVisible();
  await page
    .getByRole('link', { name: 'Approvals', exact: false })
    .first()
    .click();
  await expect(
    page
      .getByRole('button', { name: 'Review', exact: true })
      .filter({ hasText: 'Verified support correction' }),
  ).toBeVisible();
  await page
    .getByRole('button', { name: 'Review', exact: true })
    .filter({ hasText: 'Verified support correction' })
    .click();
  await expect(page.getByText('You requested this query.')).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Approve', exact: true }),
  ).toHaveCount(0);
  expect(errors).toEqual([]);
});
test('approval review executes once and exposes results and timeline', async ({
  page,
}) => {
  await page
    .getByRole('link', { name: 'Approvals', exact: false })
    .first()
    .click();
  await page
    .getByRole('button', { name: 'Review', exact: true })
    .first()
    .click();
  await page.getByLabel('Review note').fill('Verified WHERE target');
  await page.getByRole('button', { name: 'Approve', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Execute approved query' }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Execute approved query' }).click();
  await page.getByRole('button', { name: 'Confirm', exact: true }).click();
  await expect(page.getByText('1 row affected.')).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Execute approved query' }),
  ).toHaveCount(0);
});
test('schema, policy, health, command palette and themes work', async ({
  page,
}) => {
  await page.getByRole('link', { name: 'Clusters', exact: true }).click();
  await expect(
    page.getByRole('table', { name: 'Production clusters' }),
  ).toBeVisible();
  await page
    .getByRole('link')
    .filter({ hasText: 'commerce-primary' })
    .first()
    .click();
  await page.getByRole('tab', { name: 'Schema', exact: true }).click();
  await page
    .locator('.schema-table > summary')
    .filter({ hasText: 'users' })
    .hover();
  await page
    .getByRole('button', { name: 'Query users top 100', exact: true })
    .click();
  await expect(page.getByRole('textbox', { name: 'SQL editor' })).toBeVisible();
  await page.getByRole('tab', { name: 'Health', exact: true }).click();
  await expect(
    page.getByRole('img', { name: 'Latency over the last 24 hours' }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Check now' }).click();
  await expect(page.getByText('Health check completed')).toBeVisible();
  await page.getByRole('tab', { name: 'Policy', exact: true }).click();
  await page.getByLabel('Maximum result rows', { exact: true }).fill('250');
  await page.getByRole('button', { name: 'Review changes' }).click();
  await expect(page.getByText('maximum rows')).toHaveCount(0);
  await page.getByRole('button', { name: 'Confirm', exact: true }).click();
  await expect(page.getByText('Safety policy saved')).toBeVisible();
  await page.keyboard.press('Control+k');
  await page
    .getByRole('textbox', { name: 'Search pages and clusters' })
    .fill('History');
  await page
    .getByRole('dialog')
    .getByRole('button', { name: 'History', exact: true })
    .click();
  await expect(
    page.getByRole('heading', { name: 'Query history' }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'User menu' }).click();
  await page.getByRole('menuitem', { name: 'Dark', exact: false }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
});
test('cluster wizard tests a connection and creates the cluster', async ({
  page,
}) => {
  await page.getByRole('link', { name: 'Clusters', exact: true }).click();
  await expect(
    page.getByRole('table', { name: 'Production clusters' }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Add cluster' }).click();
  await page
    .getByRole('dialog')
    .getByRole('combobox', { name: 'Project', exact: true })
    .click();
  await page.getByRole('option', { name: 'Commerce', exact: true }).click();
  await page
    .getByLabel('Cluster name', { exact: true })
    .fill('support-staging');
  await page.getByRole('button', { name: 'Continue' }).click();
  await page
    .getByLabel('Host', { exact: true })
    .fill('support.internal.visp.dev');
  await page.getByLabel('Database', { exact: true }).fill('commerce');
  await page.getByLabel('Username', { exact: true }).fill('vda_gateway');
  await page.getByLabel('Password', { exact: true }).fill('connection-secret');
  await page.getByRole('button', { name: 'Test connection' }).click();
  await expect(page.getByText('18 ms')).toBeVisible();
  await page.getByRole('button', { name: 'Continue' }).click();
  await page.getByRole('button', { name: 'Create cluster' }).click();
  await expect(
    page.getByRole('row', { name: 'Open support-staging', exact: true }),
  ).toBeVisible();
});
test('all routes render at 768 px and capture console/overview screenshots', async ({
  page,
}) => {
  await page.screenshot({
    path: 'screenshots/overview-light.png',
    fullPage: true,
    animations: 'disabled',
  });
  await page.getByRole('link', { name: 'Clusters', exact: true }).click();
  await expect(
    page.getByRole('table', { name: 'Production clusters' }),
  ).toBeVisible();
  await page
    .getByRole('link')
    .filter({ hasText: 'commerce-primary' })
    .first()
    .click();
  await expect(page.getByText('Safe to run')).toBeVisible();
  await page.screenshot({
    path: 'screenshots/console-light.png',
    fullPage: true,
    animations: 'disabled',
  });
  await page.getByRole('button', { name: 'User menu' }).click();
  await page.getByRole('menuitem', { name: 'Dark', exact: false }).click();
  await page.screenshot({
    path: 'screenshots/console-dark.png',
    fullPage: true,
    animations: 'disabled',
  });
  await page.setViewportSize({ width: 768, height: 1024 });
  for (const name of [
    'Overview',
    'Clusters',
    'Console',
    'Approvals',
    'History',
    'Users',
    'Access',
    'Audit log',
    'Settings',
  ]) {
    await page
      .getByRole('navigation')
      .getByRole('link', { name, exact: false })
      .first()
      .click();
    await expect(page.locator('h1')).toBeVisible();
    await expect(page.getByLabel('Loading', { exact: true })).toHaveCount(0);
    await expect(page.getByText('Unable to complete this request')).toHaveCount(
      0,
    );
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  }
  await page.screenshot({
    path: 'screenshots/settings-768-dark.png',
    fullPage: true,
    animations: 'disabled',
  });
});
test('member navigation hides admin and limits results to own history', async ({
  page,
}) => {
  await page.getByRole('button', { name: 'User menu' }).click();
  await page.getByRole('menuitem', { name: 'Log out' }).click();
  await page.getByLabel('Email', { exact: true }).fill('member@visp.dev');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(
    page.getByRole('navigation', { name: 'Administration' }),
  ).toHaveCount(0);
  await page.getByRole('link', { name: 'Clusters', exact: true }).click();
  await expect(
    page.getByRole('table', { name: 'Production clusters' }),
  ).toBeVisible();
  await expect(
    page.getByRole('heading', { name: 'identity-prod', exact: true }),
  ).toHaveCount(0);
  await page.getByRole('link', { name: 'History', exact: true }).click();
  await expect(
    page.getByRole('columnheader', { name: 'User', exact: true }),
  ).toHaveCount(0);
});
test('keyboard run can be cancelled and query tabs retain drafts', async ({
  page,
}) => {
  await page.getByRole('link', { name: 'Clusters', exact: true }).click();
  await expect(
    page.getByRole('table', { name: 'Production clusters' }),
  ).toBeVisible();
  await page
    .getByRole('link')
    .filter({ hasText: 'commerce-primary' })
    .first()
    .click();
  await expect(page.getByText('Safe to run')).toBeVisible();
  await page.keyboard.press('Control+Enter');
  await expect(
    page.getByRole('button', { name: 'Cancel', exact: true }),
  ).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(
    page.getByText('Query was cancelled.', { exact: true }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'New query tab' }).click();
  await page
    .getByRole('textbox', { name: 'SQL editor' })
    .fill('SELECT id FROM users LIMIT 17');
  await page.getByRole('link', { name: 'Overview', exact: true }).click();
  await page.getByRole('link', { name: 'Clusters', exact: true }).click();
  await expect(
    page.getByRole('table', { name: 'Production clusters' }),
  ).toBeVisible();
  await page
    .getByRole('link')
    .filter({ hasText: 'commerce-primary' })
    .first()
    .click();
  await page.getByRole('tab', { name: 'Query 2' }).click();
  await expect(page.getByRole('textbox', { name: 'SQL editor' })).toContainText(
    'LIMIT 17',
  );
});
test('grant, user, and network administration mutations work', async ({
  page,
}) => {
  await page
    .getByRole('navigation', { name: 'Administration' })
    .getByRole('link', { name: 'Access', exact: true })
    .click();
  await page.getByRole('button', { name: 'Add grant' }).first().click();
  await page.getByLabel('Find user').fill('Jordan');
  await expect(page.getByText('Searching users…')).toHaveCount(0);
  await page.getByRole('combobox', { name: 'User', exact: true }).click();
  await page
    .getByRole('option', { name: 'Jordan Lee (member@visp.dev)' })
    .click();
  await page.getByRole('combobox', { name: 'Access scope' }).click();
  await page.getByRole('option', { name: 'Platform', exact: true }).click();
  await page.getByRole('button', { name: 'Grant access' }).click();
  await expect(page.getByText('Access granted', { exact: true })).toBeVisible();
  const row = page
    .getByRole('row')
    .filter({ hasText: 'Jordan Lee' })
    .filter({ hasText: 'Platform' });
  await row.getByRole('button', { name: /Actions for/ }).click();
  await page.getByRole('menuitem', { name: 'Revoke', exact: true }).click();
  await page.getByRole('button', { name: 'Confirm', exact: true }).click();
  await expect(row).toHaveCount(0);
  await page.getByRole('link', { name: 'Users', exact: true }).click();
  await page.getByRole('button', { name: 'Create user' }).click();
  await page.getByLabel('Name', { exact: true }).fill('Taylor Quinn');
  await page.getByLabel('Email', { exact: true }).fill('taylor@visp.dev');
  await page
    .getByLabel('Initial password', { exact: true })
    .fill('strong-demo-password');
  await page
    .getByRole('dialog')
    .getByRole('button', { name: 'Create user' })
    .click();
  const userRow = page.getByRole('row').filter({ hasText: 'taylor@visp.dev' });
  await expect(userRow).toBeVisible();
  await userRow
    .getByRole('button', { name: 'Actions for Taylor Quinn' })
    .click();
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  await page.getByRole('combobox', { name: 'Organization role' }).click();
  await page
    .getByRole('option', { name: 'Administrator', exact: true })
    .click();
  await page.getByRole('button', { name: 'Save user' }).click();
  await expect(userRow).toContainText('Admin');
  await userRow
    .getByRole('button', { name: 'Actions for Taylor Quinn' })
    .click();
  await page.getByRole('menuitem', { name: 'Disable', exact: true }).click();
  await page.getByRole('button', { name: 'Confirm', exact: true }).click();
  await expect(userRow).toContainText('Disabled');
  await page.getByRole('link', { name: 'Settings', exact: true }).click();
  const input = page.getByRole('textbox', {
    name: 'Add Organization allowed CIDRs',
  });
  await input.fill('bad-cidr');
  await input.press('Enter');
  await expect(
    page.getByText('Remove or correct the highlighted CIDRs before saving.'),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Remove bad-cidr' }).click();
  await input.fill('192.168.1.0/24');
  await input.press('Enter');
  await page.getByRole('button', { name: 'Review network changes' }).click();
  await page.getByRole('button', { name: 'Confirm', exact: true }).click();
  await expect(
    page.getByText('Network policy saved', { exact: true }),
  ).toBeVisible();
});
test('audit pagination loads on scroll and filters by action', async ({
  page,
}) => {
  await page.getByRole('link', { name: 'Audit log', exact: true }).click();
  const dataRows = page.locator('tbody > tr:not(.day-heading)');
  await expect(dataRows).toHaveCount(50);
  await page
    .getByRole('button', { name: 'Load more events' })
    .scrollIntoViewIfNeeded();
  await expect.poll(() => dataRows.count()).toBeGreaterThan(50);
  await page
    .getByRole('textbox', { name: 'Filter audit action' })
    .fill('query.execute');
  await expect(
    page.getByRole('row').filter({ hasText: 'query · blocked' }),
  ).toHaveCount(0);
  await expect(
    page.getByRole('row').filter({ hasText: 'query · execute' }).first(),
  ).toBeVisible();
});
test('approved requests survive account switches and the requester can execute', async ({
  page,
}) => {
  const reason = 'Correct a customer name after a verified support request.';
  await page
    .getByRole('link', { name: 'Approvals', exact: false })
    .first()
    .click();
  await page
    .getByRole('button', { name: 'Review', exact: true })
    .filter({ hasText: reason })
    .click();
  await page.getByRole('button', { name: 'Approve', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Execute approved query' }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Close approval' }).click();
  await page.getByRole('button', { name: 'User menu' }).click();
  await page.getByRole('menuitem', { name: 'Log out' }).click();
  await page.getByLabel('Email', { exact: true }).fill('member@visp.dev');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: 'Workspace overview' }),
  ).toBeVisible();
  await page
    .getByRole('link', { name: 'Approvals', exact: false })
    .first()
    .click();
  await page.getByRole('tab', { name: /^Approved/ }).click();
  await expect(
    page
      .getByRole('button', { name: 'Review', exact: true })
      .filter({ hasText: reason }),
  ).toBeVisible();
  await page
    .getByRole('button', { name: 'Review', exact: true })
    .filter({ hasText: reason })
    .click();
  await page.getByRole('button', { name: 'Execute approved query' }).click();
  await page.getByRole('button', { name: 'Confirm', exact: true }).click();
  await expect(page.getByText('1 row affected.')).toBeVisible();
});

test('hardened history and approval states show their badges and execution error', async ({
  page,
}) => {
  await page.getByRole('link', { name: 'History', exact: true }).click();
  await page.getByRole('combobox', { name: 'Query status' }).click();
  await page.getByRole('option', { name: 'running', exact: true }).click();
  await expect(
    page.locator('small.muted').filter({ hasText: 'Running' }),
  ).toHaveText('Running');
  await page.getByRole('combobox', { name: 'Query status' }).click();
  await page.getByRole('option', { name: 'unknown', exact: true }).click();
  await expect(
    page.locator('small.muted').filter({ hasText: 'Unknown' }),
  ).toBeVisible();
  await page
    .getByRole('link', { name: 'Approvals', exact: false })
    .first()
    .click();
  await page.getByRole('tab', { name: /^All/ }).click();
  await expect(page.locator('.badge.info')).toHaveText('Executing');
  const failed = page.getByRole('listitem').filter({
    has: page.locator('.badge.danger').filter({ hasText: 'failed' }),
  });
  await expect(failed).toContainText('Target database unavailable');
  await failed.getByRole('button', { name: 'Review', exact: true }).click();
  await expect(
    page.getByRole('region', { name: 'Query approval' }).getByRole('alert'),
  ).toHaveText('Target database unavailable');
  await expect(
    page.getByRole('button', { name: 'Execute approved query' }),
  ).toHaveCount(0);
});

test('connection endpoint edits require password before saving or testing', async ({
  page,
}) => {
  await page.getByRole('link', { name: 'Clusters', exact: true }).click();
  await expect(
    page.getByRole('table', { name: 'Production clusters' }),
  ).toBeVisible();
  await page
    .getByRole('link')
    .filter({ hasText: 'commerce-primary' })
    .first()
    .click();
  await expect(page.getByText('(+1 row to detect truncation)')).toBeVisible();
  await page.getByRole('tab', { name: 'Settings', exact: true }).click();
  await page.getByLabel('Host', { exact: true }).fill('changed.example');
  const password = page.getByLabel('Password', { exact: true });
  await expect(password).toHaveAttribute('required', '');
  await expect(
    page.getByText(
      'Changing the connection endpoint requires re-entering the password',
    ),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Save connection' }),
  ).toBeDisabled();
  await expect(
    page.getByRole('button', { name: 'Test connection' }),
  ).toBeDisabled();
  await password.fill('new-target-password');
  await expect(
    page.getByRole('button', { name: 'Save connection' }),
  ).toBeEnabled();
  await expect(
    page.getByRole('button', { name: 'Test connection' }),
  ).toBeEnabled();
});
test('console suggests optimizations and applies fixes without flicker', async ({
  page,
}) => {
  // Navigate in-app: a full page load resets the mock session.
  await page.getByRole('link', { name: 'Run a first query' }).click();
  const editor = page.getByRole('textbox', { name: 'SQL editor' });
  const panel = page.getByRole('region', { name: 'SQL safety analysis' });
  const hints = panel.getByLabel('Optimization suggestions');
  await editor.fill('SELECT * FROM users');
  await expect(
    hints.getByRole('button', { name: 'List columns' }),
  ).toBeEnabled();
  await expect(
    hints.getByRole('button', { name: 'Add LIMIT 100' }),
  ).toBeVisible();

  await hints.getByRole('button', { name: 'List columns' }).click();
  await expect(editor).toHaveText(
    'SELECT id, name, email, created_at FROM users',
  );
  await expect(
    hints.getByRole('button', { name: 'List columns' }),
  ).toBeHidden();

  // While re-analyzing, the previous verdict stays on screen instead of "Analyzing…".
  const seen = await page.evaluate(() => {
    const labels = new Set<string>();
    const observer = new MutationObserver(() =>
      labels.add(
        document.querySelector('.safety-panel .verdict strong')?.textContent ??
          '',
      ),
    );
    observer.observe(document.querySelector('.safety-panel')!, {
      subtree: true,
      childList: true,
      characterData: true,
    });
    (window as unknown as { __stop: () => string[] }).__stop = () => {
      observer.disconnect();
      return [...labels];
    };
    return true;
  });
  expect(seen).toBe(true);
  await editor.press('End');
  await editor.pressSequentially(" WHERE email LIKE '%@visp.dev'", {
    delay: 20,
  });
  await expect(hints).toContainText('LIKE pattern that starts with %');
  const labels = await page.evaluate(() =>
    (window as unknown as { __stop: () => string[] }).__stop(),
  );
  expect(labels).not.toContain('Analyzing…');

  await editor.fill('DELETE FROM users');
  await expect(panel.getByText('Blocked', { exact: true })).toBeVisible();
  await expect(hints).toBeHidden();
});
