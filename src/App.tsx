import { useCallback, useEffect, useRef, useState } from 'react';
import type {
  BootEntry,
  DownloadProgress,
  DriveInfo,
  FlashProgress,
  FlashResult,
  ImageInfo,
  Settings,
} from '@shared/types';
import { BRAND } from '@shared/brand';
import { pyro } from './api';
import { baseName, formatBytes, formatEta, formatSpeed } from './format';
import { FlameIcon, GearIcon, UsbIcon } from './icons';
import { LANGS, setLang, t, type Lang } from './i18n';

const GLOBAL = '__global';

type SourceOpts = { temp?: string; clone?: string };

function cloneImage(d: DriveInfo): ImageInfo {
  return {
    path: d.device,
    name: `Clone of ${d.description}`,
    fileSize: d.size,
    uncompressedSize: d.size,
    compression: 'none',
    bmapPath: null,
  };
}

export default function App() {
  const [image, setImage] = useState<ImageInfo | null>(null);
  const [cloneSource, setCloneSource] = useState<string | null>(null);
  const [tempPath, setTempPath] = useState<string | null>(null);
  const [drives, setDrives] = useState<DriveInfo[]>([]);
  const [devices, setDevices] = useState<string[]>([]);
  const [bootConfigs, setBootConfigs] = useState<string[]>([]);
  const [progress, setProgress] = useState<Record<string, FlashProgress>>({});
  const [results, setResults] = useState<FlashResult[] | null>(null);
  const [flashing, setFlashing] = useState(false);
  const [showSystem, setShowSystem] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [editBoot, setEditBoot] = useState(false);
  const [editing, setEditing] = useState<{ dir: string } | null>(null);
  const [editAddNonce, setEditAddNonce] = useState(0);
  const editingRef = useRef<{ dir: string } | null>(null);
  editingRef.current = editing;
  const [settings, setSettings] = useState<Settings>({
    validate: true,
    notifications: true,
    language: 'en',
  });
  const [showSettings, setShowSettings] = useState(false);
  // Apply the chosen language before rendering any translated strings.
  setLang((settings.language as Lang) || 'en');

  const refreshDrives = useCallback(async () => {
    const list = await pyro.listDrives();
    setDrives(list);
    setDevices((cur) => cur.filter((d) => list.some((x) => x.device === d)));
  }, []);

  useEffect(() => {
    refreshDrives();
    pyro.getSettings().then(setSettings);
    const offDrives = pyro.onDrivesChanged(setDrives);
    const offProgress = pyro.onFlashProgress((p) => {
      setProgress((prev) => ({ ...prev, [p.device ?? GLOBAL]: p }));
      // The helper signals the editor is ready and sends the mountpoint.
      if (p.phase === 'editing' && p.message) {
        setEditing({ dir: p.message });
      }
    });
    return () => {
      offDrives();
      offProgress();
    };
  }, [refreshDrives]);

  const handleSource = useCallback(
    (img: ImageInfo, opts: SourceOpts = {}) => {
      setTempPath((old) => {
        if (old && old !== opts.temp) void pyro.forgetTemp(old);
        return opts.temp ?? null;
      });
      setImage(img);
      setCloneSource(opts.clone ?? null);
      setResults(null);
    },
    [],
  );

  const clearSource = () => {
    if (tempPath) void pyro.forgetTemp(tempPath);
    setImage(null);
    setCloneSource(null);
    setTempPath(null);
    setResults(null);
  };

  const addBootConfigs = useCallback((paths: string[]) => {
    setBootConfigs((prev) => Array.from(new Set([...prev, ...paths])));
  }, []);

  // Route OS file drops to the zone under the cursor (image vs boot-config).
  useEffect(() => {
    const off = pyro.onFileDrop((paths, pos) => {
      if (paths.length === 0) return;
      // While the boot editor is open, dropped files go onto the partition.
      if (editingRef.current) {
        pyro
          .bootAdd(editingRef.current.dir, paths)
          .then(() => setEditAddNonce((n) => n + 1));
        return;
      }
      let zone: string | null = null;
      if (pos) {
        const dpr = window.devicePixelRatio || 1;
        const el = document.elementFromPoint(pos.x / dpr, pos.y / dpr);
        zone = el?.closest('[data-drop]')?.getAttribute('data-drop') ?? null;
      }
      if (zone === 'bootconfig') {
        addBootConfigs(paths);
      } else {
        pyro.inspectImage(paths[0]).then((img) => img && handleSource(img));
      }
    }, setDragging);
    return off;
  }, [addBootConfigs, handleSource]);

  const updateSetting = (patch: Partial<Settings>) => {
    setSettings((prev) => {
      const next = { ...prev, ...patch };
      void pyro.setSettings(next);
      return next;
    });
  };

  const selectedDrives = drives.filter((d) => devices.includes(d.device));
  const canFlash = !!image && devices.length > 0 && !flashing;

  const flash = async () => {
    if (!image || devices.length === 0) return;
    setFlashing(true);
    setResults(null);
    setProgress({
      [GLOBAL]: {
        phase: 'starting',
        fraction: 0,
        bytes: 0,
        totalBytes: null,
        speed: 0,
        eta: null,
      },
    });
    try {
      const res = await pyro.startFlash({
        image,
        devices,
        validate: settings.validate,
        bootConfigFiles: bootConfigs,
        editBoot,
      });
      setResults(res);
      if (settings.notifications) await notifyDone(res);
    } catch (err) {
      setResults(
        devices.map((device) => ({
          ok: false,
          device,
          bytesWritten: 0,
          error: err instanceof Error ? err.message : String(err),
        })),
      );
    } finally {
      setFlashing(false);
      setProgress({});
      setEditing(null);
      refreshDrives();
    }
  };

  const reset = () => {
    clearSource();
    setDevices([]);
    setBootConfigs([]);
  };

  return (
    <div className="app">
      <header className="topbar">
        <div className="logo">
          <FlameIcon />
        </div>
        <div className="brand">
          {BRAND.name}
          <small>{t('tagline')}</small>
        </div>
        <div className="spacer" />
        <button
          className="icon-btn"
          title="Settings"
          onClick={() => setShowSettings(true)}
        >
          <GearIcon />
        </button>
      </header>

      {showSettings && (
        <SettingsModal
          settings={settings}
          onChange={updateSetting}
          onClose={() => setShowSettings(false)}
        />
      )}

      {results ? (
        <ResultView results={results} onAgain={reset} />
      ) : editing ? (
        <BootEditor
          dir={editing.dir}
          refreshKey={editAddNonce}
          onAddFiles={async () => {
            const files = await pyro.selectBootConfigFiles();
            if (files.length) {
              await pyro.bootAdd(editing.dir, files);
              setEditAddNonce((n) => n + 1);
            }
          }}
          onDone={() => pyro.finishEdit()}
        />
      ) : (
        <>
          <main className="stage">
            <section className="step" data-drop="image">
              <span className="step-num">1 · {t('step.source')}</span>
              <h2>{t('source.title')}</h2>
              <div className="body">
                <SourceSelector
                  image={image}
                  drives={drives}
                  dragging={dragging}
                  onSource={handleSource}
                  onClear={clearSource}
                />
              </div>
            </section>

            <section className={`step ${!image ? 'disabled' : ''}`}>
              <span className="step-num">2 · {t('step.drive')}</span>
              <h2>{t('drive.title')}</h2>
              <div className="body">
                <DriveList
                  drives={drives.filter((d) => d.device !== cloneSource)}
                  selected={devices}
                  showSystem={showSystem}
                  requiredSize={image?.uncompressedSize ?? null}
                  onToggle={(dev) =>
                    setDevices((cur) =>
                      cur.includes(dev)
                        ? cur.filter((d) => d !== dev)
                        : [...cur, dev],
                    )
                  }
                  onToggleSystem={() => setShowSystem((s) => !s)}
                />
              </div>
            </section>

            <section className={`step ${!image ? 'disabled' : ''}`}>
              <span className="step-num">3 · {t('step.options')}</span>
              <h2>{t('options.title')}</h2>
              <div className="body" style={{ justifyContent: 'flex-start', paddingTop: 6 }}>
                <BootConfigList
                  files={bootConfigs}
                  dragging={dragging}
                  onAdd={async () => addBootConfigs(await pyro.selectBootConfigFiles())}
                  onRemove={(f) =>
                    setBootConfigs((cur) => cur.filter((x) => x !== f))
                  }
                />
                <label className="edit-toggle">
                  <input
                    type="checkbox"
                    checked={editBoot}
                    onChange={(e) => setEditBoot(e.target.checked)}
                  />
                  {t('options.editToggle')}
                </label>
              </div>
            </section>

            <section className={`step flash-step ${!canFlash && !flashing ? 'disabled' : ''}`}>
              <span className="step-num">4 · {t('step.flash')}</span>
              <h2>{t('flash.ready')}</h2>
              <div className="body">
                {flashing ? (
                  <>
                    <div style={{ width: '100%' }}>
                      <FlashProgressView
                        devices={devices}
                        drives={drives}
                        progress={progress}
                      />
                    </div>
                    <button
                      className="btn ghost"
                      style={{ marginTop: 12 }}
                      onClick={() => pyro.cancelFlash()}
                    >
                      {t('flash.cancel')}
                    </button>
                  </>
                ) : (
                  <>
                    <button className="btn-flash" disabled={!canFlash} onClick={flash}>
                      <FlameIcon />
                      <span>{t('flash.button')}</span>
                    </button>
                    <p className="muted" style={{ fontSize: 12, marginTop: 10 }}>
                      {canFlash
                        ? selectedDrives.length > 1
                          ? t('flash.readyN', { n: selectedDrives.length })
                          : t('flash.ready')
                        : t('flash.notReady')}
                    </p>
                  </>
                )}
              </div>
            </section>
          </main>
        </>
      )}
    </div>
  );
}

async function notifyDone(results: FlashResult[]) {
  const ok = results.every((r) => r.ok);
  await pyro.notify(
    ok ? t('notify.completeTitle') : t('notify.failedTitle'),
    ok
      ? t('notify.completeBody', { n: results.length })
      : results.find((r) => !r.ok)?.error ?? t('notify.failedTitle'),
  );
}

function SourceSelector({
  image,
  drives,
  dragging,
  onSource,
  onClear,
}: {
  image: ImageInfo | null;
  drives: DriveInfo[];
  dragging: boolean;
  onSource: (img: ImageInfo, opts?: SourceOpts) => void;
  onClear: () => void;
}) {
  const [mode, setMode] = useState<'idle' | 'url' | 'clone'>('idle');
  const [url, setUrl] = useState('');
  const [dl, setDl] = useState<DownloadProgress | null>(null);
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (image) {
    return (
      <div className="pick">
        <div className="name">{image.name}</div>
        <div className="sub">
          {formatBytes(image.fileSize)}
          {image.compression !== 'none' ? ` · ${image.compression}` : ''}
          {image.bmapPath ? ' · bmap ⚡' : ''}
        </div>
        <button className="link" onClick={onClear}>
          {t('source.change')}
        </button>
      </div>
    );
  }

  if (dl) {
    return (
      <div style={{ width: '100%' }}>
        <p className="muted">{t('source.downloading')}</p>
        <div className="progress">
          <i style={{ width: `${Math.round(dl.fraction * 100)}%` }} />
        </div>
        <div className="stat-row">
          <span>{formatBytes(dl.bytes)}</span>
          {dl.speed > 0 && <span>{formatSpeed(dl.speed)}</span>}
          {dl.eta != null && <span>{t('eta.left', { t: formatEta(dl.eta) })}</span>}
        </div>
      </div>
    );
  }

  const pickFile = async () => {
    const img = await pyro.selectImage();
    if (img) onSource(img);
  };

  const fetchUrl = async () => {
    if (!url.trim()) return;
    setError(null);
    setChecking(true);
    try {
      const info = await pyro.inspectUrl(url.trim());
      if (info.compression === 'zip') {
        // Zip can't be streamed (needs random access) — download to a temp file.
        setChecking(false);
        setDl({ fraction: 0, bytes: 0, totalBytes: null, speed: 0, eta: null });
        const off = pyro.onDownloadProgress(setDl);
        try {
          const img = await pyro.downloadImage(url.trim());
          onSource(img, { temp: img.path });
        } finally {
          off();
          setDl(null);
        }
      } else {
        // Stream directly to the device during flashing (write-while-download).
        onSource(info);
      }
      setMode('idle');
      setUrl('');
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setChecking(false);
    }
  };

  return (
    <div style={{ width: '100%' }}>
      <div className="source-tabs">
        <button
          className={`tab ${mode === 'idle' ? 'active' : ''}`}
          onClick={() => setMode('idle')}
        >
          {t('source.file')}
        </button>
        <button
          className={`tab ${mode === 'url' ? 'active' : ''}`}
          onClick={() => setMode('url')}
        >
          {t('source.url')}
        </button>
        <button
          className={`tab ${mode === 'clone' ? 'active' : ''}`}
          onClick={() => setMode('clone')}
        >
          {t('source.clone')}
        </button>
      </div>

      {mode === 'idle' && (
        <div style={{ textAlign: 'center', marginTop: 12 }}>
          <button className="btn" onClick={pickFile}>
            {t('source.select')}
          </button>
          <p
            className="muted"
            style={{ fontSize: 12, marginTop: 8, color: dragging ? 'var(--ember-2)' : undefined }}
          >
            {dragging ? t('source.dropHint') : t('source.dragHint')}
          </p>
        </div>
      )}

      {mode === 'url' && (
        <div style={{ display: 'grid', gap: 8, marginTop: 12 }}>
          <input
            className="url-input"
            placeholder={t('source.urlPlaceholder')}
            value={url}
            autoFocus
            onChange={(e) => setUrl(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && fetchUrl()}
          />
          {error && <span style={{ color: 'var(--bad)', fontSize: 12 }}>{error}</span>}
          <button className="btn primary" onClick={fetchUrl} disabled={checking}>
            {checking ? t('source.checking') : t('source.fetch')}
          </button>
        </div>
      )}

      {mode === 'clone' && (
        <div style={{ marginTop: 12 }}>
          <p className="muted" style={{ fontSize: 12, marginBottom: 6 }}>
            {t('source.cloneHint')}
          </p>
          {drives.length === 0 ? (
            <p className="muted">{t('source.noDrives')}</p>
          ) : (
            <div style={{ display: 'grid', gap: 6 }}>
              {drives.map((d) => (
                <DriveRow
                  key={d.device}
                  drive={d}
                  selected={false}
                  onSelect={() => onSource(cloneImage(d), { clone: d.device })}
                />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function BootConfigList({
  files,
  dragging,
  onAdd,
  onRemove,
}: {
  files: string[];
  dragging: boolean;
  onAdd: () => void;
  onRemove: (f: string) => void;
}) {
  return (
    <div
      data-drop="bootconfig"
      className="bootconfig"
      style={{ borderColor: dragging ? 'var(--ember)' : undefined }}
    >
      {files.length > 0 ? (
        <>
          <div className="sub" style={{ marginBottom: 4 }}>
            {t('options.files')}
          </div>
          {files.map((f) => (
            <div key={f} className="bootconfig-row">
              <span className="name" title={f}>
                {baseName(f)}
              </span>
              <button className="link" onClick={() => onRemove(f)}>
                ✕
              </button>
            </div>
          ))}
          <button className="link" onClick={onAdd}>
            {t('options.addMore')}
          </button>
        </>
      ) : (
        <button className="link" onClick={onAdd}>
          {t('options.add')}
        </button>
      )}
    </div>
  );
}

function DriveList({
  drives,
  selected,
  showSystem,
  requiredSize,
  onToggle,
  onToggleSystem,
}: {
  drives: DriveInfo[];
  selected: string[];
  showSystem: boolean;
  requiredSize: number | null;
  onToggle: (device: string) => void;
  onToggleSystem: () => void;
}) {
  const removable = drives.filter((d) => !d.isSystem);
  const system = drives.filter((d) => d.isSystem);
  const visible = showSystem ? drives : removable;

  return (
    <div style={{ width: '100%', display: 'grid', gap: 8 }}>
      {visible.length === 0 ? (
        <p className="muted">
          <UsbIcon /> {t('drive.none')}
        </p>
      ) : (
        visible.map((d) => (
          <DriveRow
            key={d.device}
            drive={d}
            selected={selected.includes(d.device)}
            tooSmall={requiredSize != null && d.size > 0 && d.size < requiredSize}
            onSelect={() => onToggle(d.device)}
          />
        ))
      )}
      {system.length > 0 && (
        <button className="link" onClick={onToggleSystem} style={{ marginTop: 4 }}>
          {showSystem
            ? t('drive.hideInternal')
            : t('drive.showInternal', { n: system.length })}
        </button>
      )}
    </div>
  );
}

function DriveRow({
  drive,
  selected,
  tooSmall = false,
  onSelect,
}: {
  drive: DriveInfo;
  selected: boolean;
  tooSmall?: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      className="pick"
      disabled={tooSmall}
      style={{
        borderColor: selected ? 'var(--ember)' : undefined,
        boxShadow: selected ? '0 0 0 1px var(--ember) inset' : undefined,
        display: 'flex',
        alignItems: 'center',
        gap: 10,
        opacity: tooSmall ? 0.45 : undefined,
        cursor: tooSmall ? 'not-allowed' : undefined,
      }}
      onClick={onSelect}
    >
      <span
        aria-hidden
        style={{
          width: 16,
          height: 16,
          flex: '0 0 16px',
          borderRadius: 4,
          border: `1.5px solid ${selected ? 'var(--ember)' : 'var(--text-faint)'}`,
          background: selected ? 'var(--ember)' : 'transparent',
          display: 'grid',
          placeItems: 'center',
          color: '#1a1206',
          fontSize: 12,
          lineHeight: 1,
        }}
      >
        {selected ? '✓' : ''}
      </span>
      <span style={{ minWidth: 0 }}>
        <div className="name">{drive.description}</div>
        <div className="sub">
          {drive.device} · {formatBytes(drive.size)}
          {drive.busType ? ` · ${drive.busType}` : ''}
          {drive.isSystem ? ` · ⚠ ${t('drive.system')}` : ''}
          {tooSmall ? ` · ⚠ ${t('drive.tooSmall')}` : ''}
        </div>
      </span>
    </button>
  );
}

function FlashProgressView({
  devices,
  drives,
  progress,
}: {
  devices: string[];
  drives: DriveInfo[];
  progress: Record<string, FlashProgress>;
}) {
  const label = (dev: string) =>
    drives.find((d) => d.device === dev)?.description ?? dev;
  const list = devices.length > 0 ? devices : Object.keys(progress);
  return (
    <div style={{ display: 'grid', gap: 8 }}>
      {list.map((dev) => {
        const p = progress[dev] ?? progress[GLOBAL];
        if (!p) return null;
        return (
          <div key={dev}>
            {devices.length > 1 && (
              <div className="muted" style={{ fontSize: 12 }}>
                {label(dev)}
              </div>
            )}
            <div className="progress">
              <i style={{ width: `${Math.round(p.fraction * 100)}%` }} />
            </div>
            <div className="stat-row">
              <span>
                <b>{t(`phase.${p.phase}`)}</b>
              </span>
              <span>{Math.round(p.fraction * 100)}%</span>
              {p.speed > 0 && <span>{formatSpeed(p.speed)}</span>}
              {p.eta != null && (
                <span>{t('eta.left', { t: formatEta(p.eta) })}</span>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

function SettingsModal({
  settings,
  onChange,
  onClose,
}: {
  settings: Settings;
  onChange: (patch: Partial<Settings>) => void;
  onClose: () => void;
}) {
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2 style={{ marginTop: 0 }}>{t('settings.title')}</h2>
        <Toggle
          label={t('settings.validate')}
          checked={settings.validate}
          onChange={(v) => onChange({ validate: v })}
        />
        <Toggle
          label={t('settings.notifications')}
          checked={settings.notifications}
          onChange={(v) => onChange({ notifications: v })}
        />
        <div className="toggle-row">
          <span>{t('settings.language')}</span>
          <div style={{ display: 'flex', gap: 6 }}>
            {LANGS.map((l) => (
              <button
                key={l.code}
                className={`tab ${settings.language === l.code ? 'active' : ''}`}
                style={{ padding: '5px 12px', borderRadius: 7 }}
                onClick={() => onChange({ language: l.code })}
              >
                {l.label}
              </button>
            ))}
          </div>
        </div>
        <div className="modal-footer">
          <span className="muted" style={{ fontSize: 12 }}>
            {BRAND.name} v{BRAND.version}
          </span>
          <span style={{ flex: 1 }} />
          <button className="link" onClick={() => pyro.openExternal(BRAND.repository)}>
            GitHub
          </button>
          <button className="btn" onClick={onClose}>
            {t('settings.done')}
          </button>
        </div>
      </div>
    </div>
  );
}

function Toggle({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="toggle-row">
      <span>{label}</span>
      <button
        className={`toggle ${checked ? 'on' : ''}`}
        role="switch"
        aria-checked={checked}
        onClick={() => onChange(!checked)}
      >
        <i />
      </button>
    </div>
  );
}

function BootEditor({
  dir,
  refreshKey,
  onAddFiles,
  onDone,
}: {
  dir: string;
  refreshKey: number;
  onAddFiles: () => void;
  onDone: () => void;
}) {
  const [entries, setEntries] = useState<BootEntry[]>([]);
  const [open, setOpen] = useState<{ name: string; content: string } | null>(null);
  const [renaming, setRenaming] = useState<{ name: string; value: string } | null>(
    null,
  );
  const [finishing, setFinishing] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const join = (name: string) => `${dir}/${name}`;
  const refresh = useCallback(() => {
    pyro
      .bootList(dir)
      .then(setEntries)
      .catch((e) => setErr(String(e)));
  }, [dir]);
  useEffect(() => {
    refresh();
  }, [refresh, refreshKey]);

  const openFile = async (name: string) => {
    try {
      const content = await pyro.bootReadText(join(name));
      setOpen({ name, content });
      setErr(null);
    } catch (e) {
      setErr(`Can't open ${name}: ${e instanceof Error ? e.message : e}`);
    }
  };
  const save = async () => {
    if (!open) return;
    await pyro.bootWriteText(join(open.name), open.content);
    setOpen(null);
    refresh();
  };
  const doRename = async () => {
    if (!renaming || !renaming.value.trim()) return;
    await pyro.bootRename(join(renaming.name), join(renaming.value.trim()));
    setRenaming(null);
    refresh();
  };
  const del = async (name: string) => {
    await pyro.bootDelete(join(name));
    refresh();
  };

  return (
    <main className="editor">
      <div className="editor-head">
        <div>
          <h2 style={{ margin: 0 }}>{t('editor.title')}</h2>
          <span className="muted" style={{ fontSize: 12 }}>
            {t('editor.subtitle')}
          </span>
        </div>
        <span style={{ flex: 1 }} />
        <button className="btn" onClick={onAddFiles}>
          {t('editor.add')}
        </button>
        <button
          className="btn primary"
          disabled={finishing}
          onClick={() => {
            setFinishing(true);
            onDone();
          }}
        >
          {finishing ? t('editor.ejecting') : t('editor.done')}
        </button>
      </div>

      {err && <p style={{ color: 'var(--bad)', fontSize: 13 }}>{err}</p>}

      {open ? (
        <div className="editor-file">
          <div className="editor-file-head">
            <b>{open.name}</b>
            <span style={{ flex: 1 }} />
            <button className="btn ghost" onClick={() => setOpen(null)}>
              {t('editor.cancel')}
            </button>
            <button className="btn primary" onClick={save}>
              {t('editor.save')}
            </button>
          </div>
          <textarea
            className="editor-textarea"
            value={open.content}
            spellCheck={false}
            onChange={(e) => setOpen({ ...open, content: e.target.value })}
          />
        </div>
      ) : (
        <div className="editor-list">
          {entries.length === 0 ? (
            <p className="muted">{t('editor.empty')}</p>
          ) : (
            entries.map((en) => (
              <div className="editor-row" key={en.name}>
                {renaming?.name === en.name ? (
                  <>
                    <input
                      className="url-input"
                      value={renaming.value}
                      autoFocus
                      onChange={(e) =>
                        setRenaming({ name: en.name, value: e.target.value })
                      }
                      onKeyDown={(e) => e.key === 'Enter' && doRename()}
                    />
                    <button className="link" onClick={doRename}>
                      OK
                    </button>
                    <button className="link" onClick={() => setRenaming(null)}>
                      ✕
                    </button>
                  </>
                ) : (
                  <>
                    <span className="editor-name" title={en.name}>
                      {en.isDir ? '📁 ' : ''}
                      {en.name}
                    </span>
                    <span className="muted" style={{ fontSize: 12 }}>
                      {en.isDir ? '' : formatBytes(en.size)}
                    </span>
                    {!en.isDir && (
                      <button className="link" onClick={() => openFile(en.name)}>
                        {t('editor.edit')}
                      </button>
                    )}
                    <button
                      className="link"
                      onClick={() => setRenaming({ name: en.name, value: en.name })}
                    >
                      {t('editor.rename')}
                    </button>
                    <button className="link" onClick={() => del(en.name)}>
                      {t('editor.delete')}
                    </button>
                  </>
                )}
              </div>
            ))
          )}
        </div>
      )}
    </main>
  );
}

function ResultView({
  results,
  onAgain,
}: {
  results: FlashResult[];
  onAgain: () => void;
}) {
  const ok = results.every((r) => r.ok);
  return (
    <main
      className="stage"
      style={{ flexDirection: 'column', alignItems: 'center', justifyContent: 'center' }}
    >
      <h1 style={{ color: ok ? 'var(--good)' : 'var(--bad)' }}>
        {ok ? t('result.complete') : t('result.failed')}
      </h1>
      {results.map((r) => (
        <p key={r.device} className="muted">
          {r.device}:{' '}
          {r.ok
            ? t('result.written', { bytes: formatBytes(r.bytesWritten) })
            : r.error}
        </p>
      ))}
      <button className="btn primary" onClick={onAgain} style={{ marginTop: 16 }}>
        {t('result.again')}
      </button>
    </main>
  );
}
