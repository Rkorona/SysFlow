'use strict';

/**
 * SysFlow module rescue console.
 *
 * Plain HTML + JS on purpose: it ships inside the module zip and is the only GUI available when the
 * APK cannot be installed, updated or launched. No build step, no framework, no npm.
 *
 * The KernelSU WebView injects a global `ksu` object; `ksu.exec(command, optionsJson, callbackName)`
 * calls back into a global function with (errno, stdout, stderr). This mirrors what the official
 * `kernelsu` npm wrapper does, without the dependency.
 */

const BIN_CANDIDATES = ['/data/adb/modules/sysflow/bin/sysflow', 'sysflow'];
const PROTOCOL_VERSION = 1;

let cachedBin = null;
let callbackSeq = 0;
let busy = null;

/* ------------------------------------------------------------------ shell */

function execShell(command) {
  return new Promise((resolve, reject) => {
    if (typeof window.ksu === 'undefined') {
      reject(new Error('当前环境没有 KernelSU 的 exec API，请在模块管理器中打开本页面。'));
      return;
    }
    const callbackName = `sysflow_cb_${Date.now()}_${callbackSeq++}`;
    window[callbackName] = (errno, stdout, stderr) => {
      delete window[callbackName];
      resolve({ errno, stdout: stdout || '', stderr: stderr || '' });
    };
    try {
      window.ksu.exec(command, '{}', callbackName);
    } catch (cause) {
      delete window[callbackName];
      reject(cause);
    }
  });
}

function quote(value) {
  return `'${String(value).replace(/'/g, `'\\''`)}'`;
}

async function resolveBinary() {
  if (cachedBin) return cachedBin;
  for (const candidate of BIN_CANDIDATES) {
    const probe = candidate.startsWith('/')
      ? `[ -x ${quote(candidate)} ]`
      : `command -v ${quote(candidate)}`;
    try {
      const result = await execShell(probe);
      if (result.errno === 0) {
        cachedBin = candidate;
        return candidate;
      }
    } catch {
      // A restricted shell may not support `command -v`; try the next candidate.
    }
  }
  throw new Error('找不到 sysflow 二进制，请确认模块已安装并重启。');
}

/** Runs the CLI and unwraps the protocol envelope. */
async function run(args) {
  const bin = await resolveBinary();
  const command = [bin, ...args].map(quote).join(' ');
  const { errno, stdout, stderr } = await execShell(command);
  const text = stdout.trim();
  if (!text) {
    const detail = stderr.trim();
    if (/not found|No such file/i.test(detail)) throw new Error('找不到 sysflow 二进制，请重新安装模块。');
    if (/permission|Permission denied|root/i.test(detail)) throw new Error('没有 root 权限，请在 KernelSU / APatch 中授权。');
    throw new Error(detail || `sysflow 退出码 ${errno}`);
  }
  let envelope;
  try {
    envelope = JSON.parse(text);
  } catch {
    throw new Error('sysflow 返回了无法解析的输出。');
  }
  if (!envelope.ok) {
    throw new Error((envelope.error && envelope.error.msg) || '未知错误');
  }
  if (envelope.data === undefined || envelope.data === null) {
    throw new Error('sysflow 返回了空数据。');
  }
  return envelope.data;
}

/* ------------------------------------------------------------------- view */

const el = (id) => document.getElementById(id);

function setBanners(items) {
  const box = el('banners');
  if (!items.length) {
    box.hidden = true;
    box.innerHTML = '';
    return;
  }
  box.hidden = false;
  box.innerHTML = items
    .map(
      (item) =>
        `<div class="banner banner--${item.tone}">` +
        (item.title ? `<strong>${escapeHtml(item.title)}</strong>` : '') +
        `<span>${escapeHtml(item.text)}</span></div>`,
    )
    .join('');
}

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (character) => {
    switch (character) {
      case '&': return '&amp;';
      case '<': return '&lt;';
      case '>': return '&gt;';
      case '"': return '&quot;';
      default: return '&#39;';
    }
  });
}

function renderStatus(state) {
  el('version-pill').textContent = state.caps
    ? `v${String(state.caps.sysflow_version).replace(/^v/i, '')}`
    : '—';

  const supported = state.caps
    ? Object.entries(state.caps.modules).filter(([, value]) => value.supported)
    : [];

  const rows = [
    ['协议版本', state.caps ? `v${state.caps.protocol}` : '—'],
    ['已接入模块', supported.length ? supported.map(([id]) => id).join('、') : '暂无'],
    ['安全模式', state.safe ? (state.safe.safe_mode ? '开启' : '关闭') : '—'],
    ['开机计数', state.safe ? `${state.safe.boot_count} / ${state.safe.threshold}` : '—'],
    ['快照时间', state.snapshot ? new Date(state.snapshot.ts * 1000).toLocaleString() : '—'],
  ];
  el('status-rows').innerHTML = rows
    .map(([key, value]) => `<div><dt>${key}</dt><dd>${escapeHtml(value)}</dd></div>`)
    .join('');

  const list = el('module-list');
  if (!state.caps || !Object.keys(state.caps.modules).length) {
    list.hidden = true;
    list.innerHTML = '';
  } else {
    list.hidden = false;
    list.innerHTML = Object.entries(state.caps.modules)
      .map(([id, value]) => {
        const detail = value.supported
          ? (value.features && value.features.length ? value.features.join(' · ') : '已接入')
          : (value.reason || '未接入');
        return `<li class="${value.supported ? 'is-on' : ''}"><strong>${escapeHtml(id)}</strong>` +
          `<small>${escapeHtml(detail)}</small></li>`;
      })
      .join('');
  }

  el('btn-safe').disabled = !(state.safe && state.safe.safe_mode) || busy !== null;

  const banners = [];
  if (state.safe && (state.safe.safe_mode || state.safe.rescue_marker)) {
    banners.push({
      tone: 'warn',
      title: state.safe.rescue_marker ? '检测到救援标记' : '安全模式已开启',
      text: state.safe.rescue_marker
        ? '自动调节已暂停。修复配置后可用下方按钮退出。'
        : `调节功能受限（开机计数 ${state.safe.boot_count}/${state.safe.threshold}）。`,
    });
  }
  return banners;
}

function setBusy(label) {
  busy = label;
  const badge = el('busy');
  if (label) {
    badge.hidden = false;
    badge.textContent = `正在执行：${label}…`;
  } else {
    badge.hidden = true;
    badge.textContent = '';
  }
  ['btn-safe', 'btn-check', 'btn-log', 'btn-reset', 'btn-reset-confirm', 'btn-refresh'].forEach((id) => {
    el(id).disabled = true;
  });
  if (!label) refreshAvailability();
}

let lastState = { caps: null, snapshot: null, safe: null };

function refreshAvailability() {
  el('btn-safe').disabled = !(lastState.safe && lastState.safe.safe_mode);
  ['btn-check', 'btn-log', 'btn-reset', 'btn-reset-confirm', 'btn-refresh'].forEach((id) => {
    el(id).disabled = false;
  });
}

/* ------------------------------------------------------------------ state */

async function load() {
  setBusy('读取状态');
  try {
    const [caps, snapshot, safe] = await Promise.all([run(['caps']), run(['snapshot']), run(['safe-mode', 'status'])]);
    if (caps.protocol !== PROTOCOL_VERSION) {
      throw new Error(`协议版本不匹配：模块 ${caps.protocol}，WebUI ${PROTOCOL_VERSION}。请更新模块。`);
    }
    lastState = { caps, snapshot, safe };
    const banners = renderStatus(lastState);
    setBanners(banners);
  } catch (cause) {
    setBanners([{ tone: 'error', title: '无法读取模块状态', text: cause.message || String(cause) }]);
  } finally {
    setBusy(null);
    refreshAvailability();
  }
}

async function act(label, work) {
  setBusy(label);
  try {
    const message = await work();
    if (message) setBanners([{ tone: 'ok', text: message }]);
  } catch (cause) {
    setBanners([{ tone: 'error', title: `${label}失败`, text: cause.message || String(cause) }]);
  } finally {
    setBusy(null);
    refreshAvailability();
  }
}

/* ---------------------------------------------------------------- actions */

el('btn-refresh').addEventListener('click', () => void load());

el('btn-safe').addEventListener('click', () =>
  void act('退出安全模式', async () => {
    await run(['safe-mode', 'clear']);
    await load();
    return '已退出安全模式。';
  }),
);

el('btn-check').addEventListener('click', () =>
  void act('诊断检查', async () => {
    const result = await run(['check']);
    const drift = Object.entries(result.drift || {});
    const rows = [['配置一致', result.clean ? '是' : '否']];
    drift.forEach(([id, keys]) => rows.push([`${id} 漂移`, (keys || []).join('、')]));
    Object.entries(result.errors || {}).forEach(([id, value]) => rows.push([`${id} 错误`, String(value)]));
    el('check-rows').innerHTML = rows
      .map(([key, value]) => `<div><dt>${escapeHtml(key)}</dt><dd>${escapeHtml(value)}</dd></div>`)
      .join('');
    el('check-card').hidden = false;
    return drift.length ? `检测到 ${drift.length} 项漂移。` : '配置与运行状态一致，未发现漂移。';
  }),
);

el('btn-log').addEventListener('click', () =>
  void act('查看日志', async () => {
    const result = await run(['log', '--tail', '200']);
    const lines = result.lines || [];
    el('log-title').textContent = `模块日志（最近 ${lines.length} 行）`;
    el('log-body').textContent = lines.length ? lines.join('\n') : '暂无日志';
    el('log-card').hidden = false;
    return `已载入 ${lines.length} 行日志。`;
  }),
);

el('btn-reset').addEventListener('click', () => {
  el('confirm-reset').hidden = false;
  el('btn-reset').disabled = true;
});

el('btn-reset-cancel').addEventListener('click', () => {
  el('confirm-reset').hidden = true;
  refreshAvailability();
});

el('btn-reset-confirm').addEventListener('click', () => {
  el('confirm-reset').hidden = true;
  void act('恢复默认设置', async () => {
    const result = await run(['reset', '--all']);
    const failed = Object.keys(result.errors || {}).length;
    await load();
    return failed ? `已恢复，但有 ${failed} 个模块报错。` : '全部模块已恢复默认设置。';
  });
});

void load();
