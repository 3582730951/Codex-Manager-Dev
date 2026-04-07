const { chromium } = require('playwright-core');

(async () => {
  const browser = await chromium.launch({
    executablePath: '/usr/bin/chromium-browser',
    headless: true,
    args: ['--no-sandbox', '--disable-dev-shm-usage']
  });
  const page = await browser.newPage();
  const suffix = Date.now().toString().slice(-6);

  await page.goto('http://host.docker.internal:13102', { waitUntil: 'networkidle' });
  await page.getByLabel('标识').fill(`ui-${suffix}`);
  await page.getByLabel('名称').fill(`界面租户-${suffix}`);
  await page.getByRole('button', { name: '创建租户' }).click();
  await page.waitForTimeout(5000);
  console.log('URL=' + page.url());
  const notice = await page.locator('.notice').textContent().catch(() => 'NONE');
  console.log('NOTICE=' + notice);
  console.log('BODY=' + await page.locator('body').innerText());
  await browser.close();
})();
