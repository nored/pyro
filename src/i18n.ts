// Tiny i18n: a flat key/value dictionary per language plus a `t()` helper.
// The current language is a module-level value set by <App> each render, so
// components can call `t(key)` without prop-drilling or context.

export type Lang = 'en' | 'de';

export const LANGS: ReadonlyArray<{ code: Lang; label: string }> = [
  { code: 'en', label: 'English' },
  { code: 'de', label: 'Deutsch' },
];

type Dict = Record<string, string>;

const en: Dict = {
  tagline: 'Flash OS images to USB drives & SD cards',
  'step.source': 'SOURCE',
  'step.drive': 'DRIVE',
  'step.options': 'OPTIONS',
  'step.flash': 'FLASH',
  'source.title': 'Choose a source',
  'source.file': 'File',
  'source.url': 'URL',
  'source.clone': 'Clone',
  'source.select': 'Select image…',
  'source.dragHint': 'or drag a file here',
  'source.dropHint': 'Drop the image here',
  'source.formats': '.img .iso .dmg or compressed archive',
  'source.urlPlaceholder': 'https://…/image.img.xz',
  'source.fetch': 'Fetch',
  'source.checking': 'Checking…',
  'source.cloneHint': 'Select a drive to clone from:',
  'source.noDrives': 'No drives detected',
  'source.change': 'Change',
  'source.downloading': 'Downloading…',
  'drive.title': 'Choose target(s)',
  'drive.none': 'No removable drives detected',
  'drive.showInternal': 'Show {n} internal disk(s) (unsafe)',
  'drive.hideInternal': 'Hide internal disks',
  'drive.tooSmall': 'too small for image',
  'drive.system': 'system drive',
  'options.title': 'Boot partition',
  'options.add': '+ add config file(s) to boot partition',
  'options.addMore': '+ add more',
  'options.files': 'Boot partition files',
  'options.editToggle': 'Edit boot files before ejecting',
  'flash.button': 'Flash',
  'flash.ready': 'Write & verify',
  'flash.readyN': 'Write & verify to {n} drives',
  'flash.notReady': 'Pick a source and at least one target drive',
  'flash.cancel': 'Cancel',
  'phase.starting': 'starting',
  'phase.flashing': 'flashing',
  'phase.validating': 'validating',
  'phase.configuring': 'configuring',
  'phase.editing': 'editing',
  'phase.finished': 'finished',
  'phase.failed': 'failed',
  'eta.left': '{t} left',
  'result.complete': 'Flash complete',
  'result.failed': 'Flash failed',
  'result.written': '{bytes} written & verified',
  'result.again': 'Flash another',
  'editor.title': 'Edit boot partition',
  'editor.subtitle': 'Tweak configs, rename or drop files — then eject.',
  'editor.add': 'Add file…',
  'editor.done': 'Done & eject',
  'editor.ejecting': 'Ejecting…',
  'editor.empty': 'Empty — drop files here to add them.',
  'editor.edit': 'edit',
  'editor.rename': 'rename',
  'editor.delete': 'delete',
  'editor.save': 'Save',
  'editor.cancel': 'Cancel',
  'settings.title': 'Settings',
  'settings.validate': 'Validate write — read the drive back and verify',
  'settings.notifications': 'Show a desktop notification when finished',
  'settings.language': 'Language',
  'settings.done': 'Done',
  'notify.completeTitle': 'Flash complete',
  'notify.failedTitle': 'Flash failed',
  'notify.completeBody': '{n} drive(s) written & verified',
};

const de: Dict = {
  tagline: 'Betriebssystem-Images auf USB-Sticks & SD-Karten schreiben',
  'step.source': 'QUELLE',
  'step.drive': 'LAUFWERK',
  'step.options': 'OPTIONEN',
  'step.flash': 'SCHREIBEN',
  'source.title': 'Quelle wählen',
  'source.file': 'Datei',
  'source.url': 'URL',
  'source.clone': 'Klonen',
  'source.select': 'Image auswählen…',
  'source.dragHint': 'oder Datei hierher ziehen',
  'source.dropHint': 'Image hier ablegen',
  'source.formats': '.img .iso .dmg oder komprimiertes Archiv',
  'source.urlPlaceholder': 'https://…/image.img.xz',
  'source.fetch': 'Laden',
  'source.checking': 'Prüfe…',
  'source.cloneHint': 'Laufwerk zum Klonen auswählen:',
  'source.noDrives': 'Keine Laufwerke erkannt',
  'source.change': 'Ändern',
  'source.downloading': 'Lädt herunter…',
  'drive.title': 'Ziel(e) wählen',
  'drive.none': 'Keine Wechseldatenträger erkannt',
  'drive.showInternal': '{n} interne Platte(n) anzeigen (unsicher)',
  'drive.hideInternal': 'Interne Platten ausblenden',
  'drive.tooSmall': 'zu klein für das Image',
  'drive.system': 'Systemlaufwerk',
  'options.title': 'Boot-Partition',
  'options.add': '+ Konfigurationsdatei(en) auf die Boot-Partition',
  'options.addMore': '+ weitere hinzufügen',
  'options.files': 'Dateien für Boot-Partition',
  'options.editToggle': 'Boot-Dateien vor dem Auswerfen bearbeiten',
  'flash.button': 'Schreiben',
  'flash.ready': 'Schreiben & prüfen',
  'flash.readyN': 'Auf {n} Laufwerke schreiben & prüfen',
  'flash.notReady': 'Wähle eine Quelle und mindestens ein Ziellaufwerk',
  'flash.cancel': 'Abbrechen',
  'phase.starting': 'Start',
  'phase.flashing': 'schreiben',
  'phase.validating': 'prüfen',
  'phase.configuring': 'konfigurieren',
  'phase.editing': 'bearbeiten',
  'phase.finished': 'fertig',
  'phase.failed': 'fehlgeschlagen',
  'eta.left': 'noch {t}',
  'result.complete': 'Schreiben abgeschlossen',
  'result.failed': 'Schreiben fehlgeschlagen',
  'result.written': '{bytes} geschrieben & geprüft',
  'result.again': 'Weiteres schreiben',
  'editor.title': 'Boot-Partition bearbeiten',
  'editor.subtitle': 'Configs anpassen, umbenennen oder Dateien ablegen — dann auswerfen.',
  'editor.add': 'Datei hinzufügen…',
  'editor.done': 'Fertig & auswerfen',
  'editor.ejecting': 'Wird ausgeworfen…',
  'editor.empty': 'Leer — Dateien hierher ziehen zum Hinzufügen.',
  'editor.edit': 'bearbeiten',
  'editor.rename': 'umbenennen',
  'editor.delete': 'löschen',
  'editor.save': 'Speichern',
  'editor.cancel': 'Abbrechen',
  'settings.title': 'Einstellungen',
  'settings.validate': 'Schreiben prüfen — Laufwerk zurücklesen und vergleichen',
  'settings.notifications': 'Benachrichtigung anzeigen, wenn fertig',
  'settings.language': 'Sprache',
  'settings.done': 'Fertig',
  'notify.completeTitle': 'Schreiben abgeschlossen',
  'notify.failedTitle': 'Schreiben fehlgeschlagen',
  'notify.completeBody': '{n} Laufwerk(e) geschrieben & geprüft',
};

const DICTS: Record<Lang, Dict> = { en, de };

let current: Lang = 'en';

export function setLang(lang: Lang) {
  current = DICTS[lang] ? lang : 'en';
}

export function t(key: string, vars?: Record<string, string | number>): string {
  let s = DICTS[current][key] ?? en[key] ?? key;
  if (vars) {
    for (const k of Object.keys(vars)) {
      s = s.replace(`{${k}}`, String(vars[k]));
    }
  }
  return s;
}
