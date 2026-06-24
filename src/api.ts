import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import type {
  BootEntry,
  DownloadProgress,
  DriveInfo,
  FlashProgress,
  FlashRequest,
  FlashResult,
  HttpAuth,
  ImageInfo,
  PyroApi,
  Settings,
} from '@shared/types';

/**
 * Frontend adapter over the Tauri Rust backend. Keeps a stable PyroApi surface
 * so UI components don't care about the IPC mechanism.
 */
export const pyro: PyroApi = {
  listDrives: () => invoke<DriveInfo[]>('list_drives'),

  onDrivesChanged: (cb) => {
    const unlisten = listen<DriveInfo[]>('drives-changed', (e) => cb(e.payload));
    return () => {
      void unlisten.then((f) => f());
    };
  },

  selectImage: () => invoke<ImageInfo | null>('select_image'),

  inspectImage: (path: string) =>
    invoke<ImageInfo | null>('inspect_image', { path }),

  inspectUrl: (url: string, auth?: HttpAuth | null) =>
    invoke<ImageInfo>('inspect_url', { url, auth: auth ?? null }),

  downloadImage: (url: string, auth?: HttpAuth | null) =>
    invoke<ImageInfo>('download_image', { url, auth: auth ?? null }),

  addRecentUrl: (url: string) => invoke<string[]>('add_recent_url', { url }),

  onDownloadProgress: (cb) => {
    const unlisten = listen<DownloadProgress>('download-progress', (e) =>
      cb(e.payload),
    );
    return () => {
      void unlisten.then((f) => f());
    };
  },

  forgetTemp: (path: string) => invoke<void>('forget_temp', { path }),

  selectBootConfigFiles: () =>
    invoke<string[]>('select_boot_config_files'),

  startFlash: (req: FlashRequest) =>
    invoke<FlashResult[]>('start_flash', { req }),

  cancelFlash: () => invoke<void>('cancel_flash'),

  finishEdit: () => invoke<void>('finish_edit'),

  choosePartition: (path: string) => invoke<void>('choose_partition', { path }),

  bootList: (dir: string) => invoke<BootEntry[]>('boot_list', { dir }),

  bootReadText: (path: string) => invoke<string>('boot_read_text', { path }),

  bootWriteText: (path: string, content: string) =>
    invoke<void>('boot_write_text', { path, content }),

  bootRename: (from: string, to: string) =>
    invoke<void>('boot_rename', { from, to }),

  bootDelete: (path: string) => invoke<void>('boot_delete', { path }),

  bootAdd: (dir: string, sources: string[]) =>
    invoke<void>('boot_add', { dir, sources }),

  onFlashProgress: (cb) => {
    const unlisten = listen<FlashProgress>('flash-progress', (e) =>
      cb(e.payload),
    );
    return () => {
      void unlisten.then((f) => f());
    };
  },

  onFileDrop: (onDrop, onHover) => {
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      const p = event.payload;
      if (p.type === 'enter' || p.type === 'over') {
        onHover(true);
      } else if (p.type === 'leave') {
        onHover(false);
      } else if (p.type === 'drop') {
        onHover(false);
        onDrop(p.paths, p.position ?? null);
      }
    });
    return () => {
      void unlisten.then((f) => f());
    };
  },

  notify: (title: string, body: string) =>
    invoke<void>('notify', { title, body }),

  getSettings: () => invoke<Settings>('get_settings'),

  setSettings: (settings: Settings) =>
    invoke<void>('set_settings', { settings }),

  openExternal: (url: string) => invoke<void>('open_external', { url }),

  osPlatform: () => invoke<string>('os_platform'),
};
