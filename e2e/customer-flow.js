// Browser end-to-end test: the whole customer journey on a phone-sized
// screen, then the admin side.
//
//   design a hoodie with Uzbek text -> preview -> order -> track
//   -> admin sees the order and downloads the 300-DPI print file
//
// Run against a started stack (docker compose up, or a local server):
//   BASE_URL=http://localhost:8080 ADMIN_USER=admin ADMIN_PASS=... node e2e/customer-flow.js
// Screenshots are written to e2e/out/.
const { chromium, devices } = require('playwright');
const fs = require('fs');
const path = require('path');

const BASE = process.env.BASE_URL || 'http://localhost:8080';
const ADMIN_USER = process.env.ADMIN_USER || 'admin';
const ADMIN_PASS = process.env.ADMIN_PASS || 'admin-dev-password';
const OUT = path.join(__dirname, 'out');

function assert(cond, message) {
  if (!cond) throw new Error('ASSERTION FAILED: ' + message);
  console.log('  ok  ' + message);
}

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch();
  const phone = await browser.newContext({ ...devices['Pixel 5'], locale: 'uz-UZ' });
  const page = await phone.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
  const shot = (name) => page.screenshot({ path: path.join(OUT, name + '.png') });

  console.log('catalog');
  await page.goto(BASE + '/');
  await page.waitForSelector('.garment');
  assert((await page.locator('.garment').count()) === 3, 'three garments are listed');
  assert((await page.locator('h2.title').innerText()) === 'Kiyimni tanlang', 'language auto-detected as Uzbek');
  await shot('1-home');

  console.log('editor');
  await page.locator('.garment').nth(1).tap();
  await page.waitForSelector('.stage canvas');
  await page.waitForTimeout(500);
  await page.locator('.tools .tool').nth(0).tap();
  await page.fill('textarea', 'Oʻzbekiston');
  await page.locator('textarea').dispatchEvent('change');
  await page.locator('.tools .tool').nth(4).tap();
  await page.locator('.picker button').nth(1).tap();
  await page.waitForTimeout(400);
  const draft = await page.evaluate(() => JSON.parse(localStorage.getItem('qd.draft')));
  assert(draft.doc.front.length === 2 && draft.doc.front[0].text === 'Oʻzbekiston', 'design has Uzbek text and a sticker');
  const priceText = (await page.locator('.sticky-bar .price').innerText()).replace(/\s/g, '');
  assert(/^\d{6,}/.test(priceText), 'live price is shown (' + priceText + ')');
  await page.evaluate(() => window.scrollTo(0, 0));
  await shot('2-editor');

  // Undo removes the sticker, redo brings it back.
  await page.locator('.topbar .icon-btn').nth(1).tap();
  assert((await page.evaluate(() => JSON.parse(localStorage.getItem('qd.draft')).doc.front.length)) === 1, 'undo works');
  await page.locator('.topbar .icon-btn').nth(2).tap();
  assert((await page.evaluate(() => JSON.parse(localStorage.getItem('qd.draft')).doc.front.length)) === 2, 'redo works');

  console.log('try-on (plain preview; the camera needs a real device)');
  await page.locator('.sticky-bar .btn-primary').tap();
  await page.waitForSelector('.seg');
  await page.locator('.seg button').nth(2).tap();
  await page.waitForSelector('.viewer.plain canvas');
  await page.waitForTimeout(500);
  await shot('3-tryon-plain');

  console.log('order');
  await page.locator('.sticky-bar .btn-primary').tap();
  await page.waitForSelector('select.select');
  await page.selectOption('select.select', 'tashkent-city');
  await page.fill('input[autocomplete="street-address"]', 'Mirobod tumani, Nukus koʻchasi 42');
  await page.fill('input[type="tel"]', '90 123 45 67');
  await page.fill('input[autocomplete="given-name"]', 'Ulugʻbek');
  await page.waitForFunction(() => /\d/.test(document.querySelector('.line.total .num').textContent));
  const total = (await page.locator('.line.total .num').innerText()).replace(/\D/g, '');
  assert(Number(total) > 200000, 'server quote is shown (' + total + ' soʻm)');
  await shot('4-order');
  // Submitting without accepting the rights statement is refused.
  await page.locator('.sticky-bar .btn-primary').tap();
  await page.waitForSelector('.toast.bad');
  await page.locator('label.check input').check();
  await page.locator('.sticky-bar .btn-primary').tap();
  await page.waitForSelector('.code');
  const code = await page.locator('.code').innerText();
  assert(/^QD-[0-9A-Z]{6}$/.test(code), 'order code received: ' + code);
  await shot('5-order-sent');

  console.log('tracking');
  await page.locator('main .btn-primary').tap();
  await page.waitForSelector('.timeline li');
  assert((await page.locator('.timeline li.now').innerText()).includes('Qabul qilindi'), 'timeline shows the order as received');
  await shot('6-track');

  console.log('admin');
  const desktop = await browser.newContext({ viewport: { width: 1200, height: 900 }, acceptDownloads: true });
  const admin = await desktop.newPage();
  admin.on('pageerror', (e) => errors.push('admin: ' + e.message));
  await admin.goto(BASE + '/admin/');
  await admin.fill('input[autocomplete="username"]', ADMIN_USER);
  await admin.fill('input[type="password"]', ADMIN_PASS);
  await admin.click('button[type="submit"]');
  await admin.waitForSelector('table.table tbody tr');
  await admin.locator('tr', { hasText: code }).click();
  // Files are rendered by a background job; the page polls until ready.
  await admin.waitForSelector('a[download]', { timeout: 60000 });
  await admin.waitForTimeout(500);
  await admin.screenshot({ path: path.join(OUT, '7-admin-order.png'), fullPage: true });
  assert((await admin.locator('.mock-pair img').count()) === 2, 'admin sees front and back mockups');
  const [download] = await Promise.all([admin.waitForEvent('download'), admin.locator('a[download]').first().click()]);
  const file = path.join(OUT, download.suggestedFilename());
  await download.saveAs(file);
  const png = fs.readFileSync(file);
  // PNG IHDR: width and height are big-endian at bytes 16 and 20.
  assert(png.readUInt32BE(16) === 3543 && png.readUInt32BE(20) === 4134, 'print file is 3543x4134 px (30x35 cm at 300 DPI): ' + download.suggestedFilename());
  await admin.locator('.btn-primary.btn-sm').first().click();
  await admin.waitForSelector('.pill.confirmed');
  assert(true, 'admin confirmed the order');

  // Reloading /track/<code> looks the order up again with the saved phone.
  await page.reload();
  await page.waitForSelector('.timeline li.now');
  assert((await page.locator('.timeline li.now').innerText()).includes('Tasdiqlandi'), 'customer sees the new status');

  assert(errors.length === 0, 'no browser errors' + (errors.length ? ': ' + errors.join(' | ') : ''));
  await browser.close();
  console.log('\nALL PASSED');
})().catch((e) => {
  console.error(e.message || e);
  process.exit(1);
});
