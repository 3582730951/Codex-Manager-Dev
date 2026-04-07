const { chromium } = require('playwright-core');

(async () => {
  const browser = await chromium.launch({
    executablePath: '/usr/bin/chromium-browser',
    headless: true,
    args: ['--no-sandbox', '--disable-dev-shm-usage']
  });
  const page = await browser.newPage();
  const suffix = Date.now().toString().slice(-6);
  const slug = `ui-${suffix}`;
  const tenantName = `界面租户-${suffix}`;
  const accountName = `界面账号-${suffix}`;

  await page.goto(process.env.BASE_URL, { waitUntil: 'networkidle' });
  await page.getByLabel('标识').fill(slug);
  await page.getByLabel('名称').fill(tenantName);
  await page.getByRole('button', { name: '创建租户' }).click();
  await page.getByText('租户已创建。').waitFor({ timeout: 15000 });

  await page.getByLabel('账号名').fill(accountName);
  await page.getByRole('button', { name: '导入到控制面' }).click();
  await page.getByText('账号已导入。').waitFor({ timeout: 15000 });

  await page.getByRole('button', { name: '恢复已有会话' }).click();
  await page.getByText('恢复任务已启动。').waitFor({ timeout: 15000 });

  console.log(JSON.stringify({ slug, tenantName, accountName }));
  await browser.close();
})();
