/** Types shared between the main process, preload, renderer and worker. */

export interface DriveInfo {
  /** Stable device path, e.g. /dev/sdb or /dev/disk4 */
  device: string;
  /** Human description / model, e.g. "SanDisk Ultra" */
  description: string;
  /** Size in bytes */
  size: number;
  /** true if removable (USB/SD) */
  isRemovable: boolean;
  /** true if this is (or contains) the OS/system disk — never flash by default */
  isSystem: boolean;
  /** true if any partition is currently mounted */
  isMounted: boolean;
  /** bus type if known: usb, sd, nvme, sata… */
  busType: string | null;
  /** mountpoints of partitions, for display */
  mountpoints: string[];
}

/** HTTP Basic Auth credentials for a URL source. */
export interface HttpAuth {
  username: string;
  password: string;
}

export interface ImageInfo {
  path: string;
  /** display name */
  name: string;
  /** compressed file size on disk, bytes */
  fileSize: number;
  /** uncompressed size if known (bytes), else null */
  uncompressedSize: number | null;
  /** detected compression format */
  compression: Compression;
  /** path to an auto-detected sibling .bmap, if any */
  bmapPath?: string | null;
  /** HTTP Basic Auth for a URL source, carried through to the flasher. */
  auth?: HttpAuth | null;
}

export type Compression =
  | 'none'
  | 'gzip'
  | 'xz'
  | 'zstd'
  | 'bzip2'
  | 'zip';

/** Erase a device and lay down a single fresh filesystem instead of flashing. */
export interface FormatSpec {
  /** 'exfat' | 'fat32' | 'ext4' */
  filesystem: string;
  /** volume label (may be empty) */
  label: string;
}

export interface FlashRequest {
  image: ImageInfo;
  /** target device paths */
  devices: string[];
  /** verify by reading back after write */
  validate: boolean;
  /** files to copy onto the boot partition root after flashing */
  bootConfigFiles: string[];
  /** keep the boot partition mounted for in-app editing before eject */
  editBoot: boolean;
  /** if set, erase & format the devices instead of writing an image */
  format?: FormatSpec | null;
}

export type FlashPhase =
  | 'starting'
  | 'flashing'
  | 'validating'
  | 'configuring'
  | 'choose'
  | 'editing'
  | 'formatting'
  | 'finished'
  | 'failed';

/** A mountable partition on the freshly written device, offered for editing. */
export interface PartitionInfo {
  /** Device path, e.g. /dev/sdb1 */
  path: string;
  /** Filesystem label, if any */
  label?: string | null;
  /** Filesystem type, e.g. vfat, ext4 */
  fstype: string;
  /** Size in bytes */
  size: number;
}

export interface FlashProgress {
  phase: FlashPhase;
  /** 0..1 for the current phase */
  fraction: number;
  /** bytes processed in current phase */
  bytes: number;
  /** total bytes for current phase, if known */
  totalBytes: number | null;
  /** bytes/sec */
  speed: number;
  /** seconds remaining estimate, if known */
  eta: number | null;
  /** per-device status messages */
  message?: string;
  /** which target device this update refers to (multi-write) */
  device?: string;
  /** partitions to choose from (only on the 'choose' phase) */
  partitions?: PartitionInfo[];
}

export interface BootEntry {
  name: string;
  isDir: boolean;
  size: number;
}

export interface Settings {
  /** Verify the write by reading the device back. */
  validate: boolean;
  /** Show a desktop notification when a flash finishes. */
  notifications: boolean;
  /** UI language code (e.g. "en", "de"). */
  language: string;
  /** Most-recently-used image URLs (newest first). */
  recentUrls?: string[];
}

export interface DownloadProgress {
  fraction: number;
  bytes: number;
  totalBytes: number | null;
  speed: number;
  eta: number | null;
}

export interface FlashResult {
  ok: boolean;
  device: string;
  bytesWritten: number;
  /** sha256 of the written/validated image, if computed */
  checksum?: string;
  error?: string;
  /** non-fatal note (e.g. no editable boot partition found) */
  warning?: string;
}

/** The API the renderer uses to talk to the Rust backend. */
export interface PyroApi {
  listDrives(): Promise<DriveInfo[]>;
  onDrivesChanged(cb: (drives: DriveInfo[]) => void): () => void;
  selectImage(): Promise<ImageInfo | null>;
  /** Build image metadata for a path (used by drag-and-drop). */
  inspectImage(path: string): Promise<ImageInfo | null>;
  /** Inspect a remote image without downloading (size + format) for streaming. */
  inspectUrl(url: string, auth?: HttpAuth | null): Promise<ImageInfo>;
  /** Download a remote image to a temp file; resolves to its metadata. */
  downloadImage(url: string, auth?: HttpAuth | null): Promise<ImageInfo>;
  /** Record a successfully-used URL; returns the updated recent list. */
  addRecentUrl(url: string): Promise<string[]>;
  onDownloadProgress(cb: (p: DownloadProgress) => void): () => void;
  /** Delete a temp file we created (e.g. a downloaded image). */
  forgetTemp(path: string): Promise<void>;
  /** Pick one or more files to copy onto the boot partition. */
  selectBootConfigFiles(): Promise<string[]>;
  startFlash(req: FlashRequest): Promise<FlashResult[]>;
  cancelFlash(): Promise<void>;
  onFlashProgress(cb: (p: FlashProgress) => void): () => void;
  /** Signal the helper that boot-file editing is done (triggers eject). */
  finishEdit(): Promise<void>;
  /** Answer a 'choose' event: which partition to mount (empty string = skip). */
  choosePartition(path: string): Promise<void>;
  bootList(dir: string): Promise<BootEntry[]>;
  bootReadText(path: string): Promise<string>;
  bootWriteText(path: string, content: string): Promise<void>;
  bootRename(from: string, to: string): Promise<void>;
  bootDelete(path: string): Promise<void>;
  bootAdd(dir: string, sources: string[]): Promise<void>;
  /** Subscribe to OS file drag-and-drop over the window. Returns an unsubscribe.
   *  `position` is in physical pixels (divide by devicePixelRatio for CSS px). */
  onFileDrop(
    onDrop: (paths: string[], position: { x: number; y: number } | null) => void,
    onHover: (over: boolean) => void,
  ): () => void;
  /** Show a desktop notification. */
  notify(title: string, body: string): Promise<void>;
  getSettings(): Promise<Settings>;
  setSettings(settings: Settings): Promise<void>;
  openExternal(url: string): Promise<void>;
  /** Host OS: "linux" | "macos" | "windows" — to hide options that don't apply. */
  osPlatform(): Promise<string>;
}
