// Copy both licence files into every publishable workspace package, and make sure each
// package's `files` list actually mentions them.
//
// WHY THIS EXISTS
// `files` lists paths relative to the PACKAGE, and npm silently omits a listed path that
// does not exist — no warning, no error, exit 0. Both licences live at the repo root, so
// every package here was publishing with NO LICENCE TEXT AT ALL while its manifest claims
// "MIT OR Apache-2.0". Verified by packing and installing physical-instruments.js: the
// installed tree was README.md, dist, package.json, wasm, worklet — and nothing else.
//
// That is worse than untidy. A dual-licensed package with neither licence in the tarball
// gives a downstream user nothing to comply with, and licence scanners flag it. The same
// defect was found and fixed in subtractive-synthesizers.js the same week.
//
// The copies are gitignored: the repo root stays the single editable original.
//
//     node scripts/dev/stage-package-files.mjs
//     node scripts/dev/stage-package-files.mjs --check   # fail instead of writing
import { copyFileSync, existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const CHECK = process.argv.includes("--check");
const LICENCES = ["LICENSE-MIT", "LICENSE-APACHE"];

/** Expand the workspace globs the same way npm does, one level deep. */
function workspaceDirs() {
  const patterns = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).workspaces ?? [];
  const out = [];
  for (const p of patterns) {
    if (p.endsWith("/*")) {
      const base = p.slice(0, -2);
      if (!existsSync(join(root, base))) continue;
      for (const entry of readdirSync(join(root, base), { withFileTypes: true })) {
        if (entry.isDirectory()) out.push(`${base}/${entry.name}`);
      }
    } else {
      out.push(p);
    }
  }
  return out;
}

const problems = [];
let staged = 0;

for (const dir of workspaceDirs()) {
  const manifest = join(root, dir, "package.json");
  if (!existsSync(manifest)) continue;
  const pkg = JSON.parse(readFileSync(manifest, "utf8"));
  if (pkg.private) continue;

  if (!Array.isArray(pkg.files)) {
    // No `files` means npm publishes the whole directory — source, configs, everything.
    // Flag it rather than guessing what belongs in the tarball.
    problems.push(`${pkg.name} has no "files" list, so it would publish its entire directory`);
    continue;
  }

  const missingFromList = LICENCES.filter((f) => !pkg.files.includes(f));
  if (missingFromList.length) {
    if (CHECK) {
      problems.push(`${pkg.name}: "files" omits ${missingFromList.join(", ")}`);
    } else {
      pkg.files = [...pkg.files, ...missingFromList];
      writeFileSync(manifest, JSON.stringify(pkg, null, 2) + "\n");
      console.log(`  ${pkg.name}: added ${missingFromList.join(", ")} to "files"`);
    }
  }

  for (const f of LICENCES) {
    const dest = join(root, dir, f);
    if (CHECK) {
      if (!existsSync(dest)) problems.push(`${pkg.name}: ${f} is not staged — run the build`);
    } else {
      copyFileSync(join(root, f), dest);
      staged++;
    }
  }
}

if (problems.length) {
  console.error("PACKAGE FILES FAIL:");
  for (const p of problems) console.error(`  ${p}`);
  process.exit(1);
}
console.log(CHECK ? "package files OK — every published package carries both licences"
                  : `staged ${staged} licence file(s)`);
