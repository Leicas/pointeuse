#!/usr/bin/env node
// Patches the just-generated `src-tauri/gen/android/` tree with Pointeuse's
// hand-edited Android customizations. Runs after `npx tauri android init`
// (CI step + locally via `npm run android:init`) because Tauri regenerates
// gen/android/ wholesale and would otherwise drop them.
//
// What this does:
//   1. Detects the generated package name (from the generated MainActivity.kt)
//      and the generated theme name (from the generated AndroidManifest.xml).
//   2. Replaces the generated AndroidManifest.xml with
//      android-overlay/AndroidManifest.xml, substituting the detected theme.
//      The overlay manifest adds: notification/alarm/boot permissions, the
//      schedule-task plugin receivers, a FileProvider, and disables default
//      WorkManager init so MainActivity can install a custom WorkerFactory.
//   3. Copies the 4 overlay Kotlin files into the generated package dir,
//      substituting `__APP_PACKAGE__` with the detected package. The overlay
//      MainActivity.kt intentionally REPLACES the generated one.
//
// Idempotent: safe to re-run on an already-patched tree.

import { readFileSync, writeFileSync, readdirSync, existsSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const genAndroid = join(repoRoot, 'src-tauri', 'gen', 'android');
const overlayDir = join(repoRoot, 'src-tauri', 'android-overlay');

if (!existsSync(genAndroid)) {
  console.error(`[patch-android-gen] gen/android/ not found at ${genAndroid} — run \`npx tauri android init\` first.`);
  process.exit(1);
}

// --- 1) Detect generated package + theme -----------------------------------
function findFile(dir, name) {
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) {
      const hit = findFile(p, name);
      if (hit) return hit;
    } else if (entry === name) {
      return p;
    }
  }
  return null;
}

const javaRoot = join(genAndroid, 'app', 'src', 'main', 'java');
const generatedMain = findFile(javaRoot, 'MainActivity.kt');
if (!generatedMain) {
  console.error('[patch-android-gen] generated MainActivity.kt not found under', javaRoot);
  process.exit(1);
}
const pkgMatch = readFileSync(generatedMain, 'utf8').match(/^package\s+([\w.]+)/m);
if (!pkgMatch) {
  console.error('[patch-android-gen] could not read package from', generatedMain);
  process.exit(1);
}
const appPackage = pkgMatch[1];
const pkgDir = dirname(generatedMain);

const manifestPath = join(genAndroid, 'app', 'src', 'main', 'AndroidManifest.xml');
const generatedManifest = readFileSync(manifestPath, 'utf8');
const themeMatch = generatedManifest.match(/android:theme="@style\/([\w.]+)"/);
const themeName = themeMatch ? themeMatch[1] : 'Theme.pointeuse';
console.log(`[patch-android-gen] package=${appPackage} theme=${themeName}`);

// --- 2) Replace AndroidManifest.xml -----------------------------------------
let overlayManifest = readFileSync(join(overlayDir, 'AndroidManifest.xml'), 'utf8');
overlayManifest = overlayManifest.replace(/@style\/[\w.]+/, `@style/${themeName}`);
writeFileSync(manifestPath, overlayManifest);
console.log('[patch-android-gen] manifest: replaced with overlay (perms + receivers + WorkManager override)');

// --- 3) Copy Kotlin overlays into the generated package dir ------------------
for (const kt of readdirSync(overlayDir).filter((f) => f.endsWith('.kt'))) {
  const src = readFileSync(join(overlayDir, kt), 'utf8').replace('__APP_PACKAGE__', appPackage);
  writeFileSync(join(pkgDir, kt), src);
  console.log(`[patch-android-gen] kotlin: ${kt} -> ${join(pkgDir, kt)}`);
}

// --- 4) Ensure the app module depends on WorkManager ------------------------
// AppWorkerFactory / MainActivity / ScheduledTaskWorkerOverride import
// androidx.work.*, and the overlay manifest disables Tauri's default
// WorkManager auto-init so MainActivity installs a custom WorkerFactory via
// on-demand initialization. The schedule-task plugin pulls WorkManager in only
// as `implementation`, so it is not exposed to the app module — without this
// the app's Kotlin fails to compile (Unresolved reference: WorkManager/Worker/
// WorkerFactory/...). Declare it directly on the app module.
const WORK_DEP = 'implementation("androidx.work:work-runtime-ktx:2.9.1")';
const appGradlePath = join(genAndroid, 'app', 'build.gradle.kts');
if (!existsSync(appGradlePath)) {
  console.error('[patch-android-gen] app/build.gradle.kts not found at', appGradlePath);
  process.exit(1);
}
let appGradle = readFileSync(appGradlePath, 'utf8');
if (appGradle.includes('androidx.work:work-runtime-ktx')) {
  console.log('[patch-android-gen] gradle: androidx.work dependency already present');
} else {
  const depsMatch = appGradle.match(/dependencies\s*\{/);
  if (!depsMatch) {
    console.error('[patch-android-gen] no `dependencies { }` block in app/build.gradle.kts');
    process.exit(1);
  }
  const insertAt = depsMatch.index + depsMatch[0].length;
  appGradle = `${appGradle.slice(0, insertAt)}\n    ${WORK_DEP}${appGradle.slice(insertAt)}`;
  writeFileSync(appGradlePath, appGradle);
  console.log('[patch-android-gen] gradle: added androidx.work dependency to app module');
}

console.log('[patch-android-gen] done.');
