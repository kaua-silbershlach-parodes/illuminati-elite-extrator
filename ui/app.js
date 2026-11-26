// Update driver for the blank template shell.
// Uses the global Tauri API (app.withGlobalTauri = true in tauri.conf.json):
// no bundler and no npm packages are needed for the interface.
const { getVersion } = window.__TAURI__.app;
const { check } = window.__TAURI__.updater;
const { relaunch } = window.__TAURI__.process;

const el = (id) => document.getElementById(id);
let pendingUpdate = null;

function setState(text) {
  el('upd-state').textContent = text;
}

function logLine(text) {
  const log = el('upd-log');
  const time = new Date().toLocaleTimeString();
  log.textContent += `[${time}] ${text}\n`;
  log.scrollTop = log.scrollHeight;
}

async function init() {
  try {
    el('version').textContent = 'v' + (await getVersion());
  } catch (err) {
    logLine('Could not read the app version: ' + err);
  }
}

async function checkNow() {
  el('btn-check').disabled = true;
  setState('checking...');
  try {
    const update = await check();
    if (!update) {
      setState('up to date');
      logLine('No update available. You are on the newest version.');
      el('btn-check').disabled = false;
      return;
    }
    pendingUpdate = update;
    setState('update available: ' + update.version);
    logLine(`Update found: ${update.version} (current: ${update.currentVersion}).`);
    el('btn-download').disabled = false;
  } catch (err) {
    setState('check failed');
    logLine('Update check failed: ' + err);
    el('btn-check').disabled = false;
  }
}

async function downloadAndInstall() {
  if (!pendingUpdate) return;
  el('btn-download').disabled = true;
  setState('downloading...');
  let downloaded = 0;
  let total = 0;
  try {
    await pendingUpdate.downloadAndInstall((event) => {
      if (event.event === 'Started') {
        total = event.data.contentLength || 0;
        logLine(`Download started (${total} bytes).`);
      } else if (event.event === 'Progress') {
        downloaded += event.data.chunkLength;
        if (total > 0) {
          el('upd-bar').style.width = Math.round((downloaded / total) * 100) + '%';
        }
      } else if (event.event === 'Finished') {
        el('upd-bar').style.width = '100%';
        logLine('Download finished.');
      }
    });
    setState('update installed - restart to apply');
    logLine('Update installed. Restart the app to apply it.');
    el('btn-restart').classList.remove('hidden');
  } catch (err) {
    setState('install failed');
    logLine('Download/install failed: ' + err);
    el('btn-download').disabled = false;
  }
}

el('btn-check').addEventListener('click', checkNow);
el('btn-download').addEventListener('click', downloadAndInstall);
el('btn-restart').addEventListener('click', () => relaunch());
init();

// Extractor driver.
const { invoke } = window.__TAURI__.core;
const dialog = window.__TAURI__.dialog;

function xlog(text) {
  const log = el('extract-log');
  const time = new Date().toLocaleTimeString();
  log.textContent += `[${time}] ${text}\n`;
  log.scrollTop = log.scrollHeight;
}

function archivePath() {
  return el('archive-path').value.trim();
}

async function browseArchive() {
  try {
    const selected = await dialog.open({ multiple: false, filters: [{ name: 'Archives', extensions: ['zip','7z','rar','tar','gz','bz2','xz','zst','tgz'] }] });
    if (selected) el('archive-path').value = selected;
  } catch (err) {
    xlog('Browse failed: ' + err);
  }
}

async function listArchive() {
  const path = archivePath();
  if (!path) { xlog('Choose an archive first.'); return; }
  try {
    const entries = await invoke('list_archive', { path });
    xlog(`Listed ${entries.length} entries:`);
    for (const entry of entries) xlog(`${entry.is_dir ? '[dir] ' : ''}${entry.name} (${entry.size} bytes)`);
  } catch (err) {
    xlog('List failed: ' + err);
  }
}

async function extractWith(command, label) {
  const path = archivePath();
  if (!path) { xlog('Choose an archive first.'); return; }
  try {
    const files = await invoke(command, { path });
    xlog(`${label}: extracted ${files.length} files.`);
    for (const file of files) xlog(`${file.path} (${file.size} bytes) sha256=${file.sha256}`);
  } catch (err) {
    xlog(label + ' failed: ' + err);
  }
}

async function registerContext() {
  try {
    xlog(await invoke('register_context_menu'));
  } catch (err) {
    xlog('Context menu registration failed: ' + err);
  }
}

el('btn-browse').addEventListener('click', browseArchive);
el('btn-list').addEventListener('click', listArchive);
el('btn-extract-here').addEventListener('click', () => extractWith('extract_here', 'Extract here'));
el('btn-extract-folder').addEventListener('click', () => extractWith('extract_to_folder', 'Extract to folder'));
el('btn-context').addEventListener('click', registerContext);

(async () => {
  try {
    const initial = await invoke('initial_path');
    if (initial) {
      el('archive-path').value = initial;
      await listArchive();
    }
  } catch (err) {
    xlog('Could not read initial archive path: ' + err);
  }
})();
