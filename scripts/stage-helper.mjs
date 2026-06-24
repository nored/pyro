// Build the privileged `pyro-helper` and stage it where Tauri's `externalBin`
// expects it (src-tauri/binaries/pyro-helper-<target-triple>), so it gets
// bundled into the AppImage/.deb/.app next to the main binary. Run automatically
// by `tauri build` via beforeBuildCommand.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

// Resolve the host target triple from rustc.
const vv = execFileSync('rustc', ['-vV'], { encoding: 'utf8' });
const triple = vv.match(/^host:\s*(.+)$/m)?.[1]?.trim();
if (!triple) {
  console.error('could not determine rust host triple from `rustc -vV`');
  process.exit(1);
}

console.log(`staging pyro-helper for ${triple}`);
execFileSync('cargo', ['build', '--release', '-p', 'pyro-helper'], {
  cwd: root,
  stdio: 'inherit',
});

const ext = process.platform === 'win32' ? '.exe' : '';
const src = join(root, 'target', 'release', `pyro-helper${ext}`);
const dstDir = join(root, 'src-tauri', 'binaries');
const dst = join(dstDir, `pyro-helper-${triple}${ext}`);
mkdirSync(dstDir, { recursive: true });
copyFileSync(src, dst);
console.log(`staged -> ${dst}`);
