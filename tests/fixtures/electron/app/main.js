// ELECTRON-A (P18 M7): an Electron app's life on Doors. Each step prints
// one "ELECTRON-A <step> ok|failed" line on the console the Terminal tab
// gives it; the run ends with PASSED or FAILED and the app quits.
const { app, BrowserWindow, ipcMain } = require('electron');
const path = require('path');
const fs = require('fs');
const os = require('os');

const results = [];
function step(name, ok, detail) {
  const line = `ELECTRON-A ${name} ${ok ? 'ok' : 'failed'}${detail ? ' ' + detail : ''}`;
  results.push(ok);
  process.stdout.write(line + '\n');
}
function finish() {
  const passed = results.length > 0 && results.every(Boolean);
  process.stdout.write(`ELECTRON-A ${passed ? 'PASSED' : 'FAILED'}\n`);
  app.exit(passed ? 0 : 1);
}
process.on('uncaughtException', (e) => {
  step('exception', false, String(e && e.stack || e).split('\n')[0]);
  finish();
});

step('main', true, `electron=${process.versions.electron} node=${process.versions.node} chrome=${process.versions.chrome}`);
step('platform', process.platform === 'win32' && os.arch() === 'x64', `${process.platform}/${os.arch()}`);
try {
  const file = path.join(os.tmpdir(), 'electron-a.txt');
  fs.writeFileSync(file, 'doors');
  step('fs', fs.readFileSync(file, 'utf8') === 'doors');
  fs.unlinkSync(file);
} catch (e) {
  step('fs', false, String(e));
}

app.whenReady().then(async () => {
  step('ready', true);
  ipcMain.handle('ping', (_e, v) => `pong ${v}`);
  const win = new BrowserWindow({
    width: 800,
    height: 600,
    title: 'Electron on Doors',
    webPreferences: { preload: path.join(__dirname, 'preload.js') },
  });
  win.webContents.on('render-process-gone', (_e, d) => {
    step('renderer', false, d.reason);
    finish();
  });
  try {
    await win.loadFile(path.join(__dirname, 'index.html'));
    step('load', true);
    const title = await win.webContents.executeJavaScript('document.title');
    step('dom', title === 'Electron on Doors', title);
    const sum = await win.webContents.executeJavaScript('compute(20)');
    step('javascript', sum === 6765, String(sum));
    const pong = await win.webContents.executeJavaScript('window.doors.ping(42)');
    step('ipc', pong === 'pong 42', pong);
    const image = await win.webContents.capturePage();
    const size = image.getSize();
    const bitmap = image.toBitmap();
    // The page paints a blue band at the top: BGRA (215, 120, 0).
    const blue = bitmap.length >= 4 && bitmap[0] > 150 && bitmap[2] < 60;
    step('paint', size.width > 0 && blue, `${size.width}x${size.height}`);
  } catch (e) {
    step('load', false, String(e));
  }
  setTimeout(finish, 3000);
});
