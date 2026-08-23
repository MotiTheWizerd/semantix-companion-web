// The Agent SDK lists every platform's sharp build as an optional dependency,
// and npm 11 installs the musl variants alongside the glibc ones even though
// only one of them can ever load here. They are not merely dead weight (16MB):
// linuxdeploy walks every ELF in the AppDir when building the AppImage, finds
// `libc.musl-x86_64.so.1` missing, and fails the whole bundle.
//
// So: after every install, delete the variants this libc cannot run. npm's
// `libc` config and the `libc` field on the packages themselves both failed to
// filter them, which is why this is a script and not a config line.
//
// Building for Alpine would mean inverting the check — the glibc copies would
// be the foreign ones then.

import { readdirSync, rmSync, existsSync } from "node:fs";
import { join } from "node:path";
import { report } from "node:process";

const IMG_DIR = join(import.meta.dirname, "node_modules", "@img");

/** True when this Node runs against musl (Alpine and friends) rather than glibc. */
function usesMusl() {
  const { glibcVersionRuntime } = report.getReport().header ?? {};
  return glibcVersionRuntime === undefined;
}

if (existsSync(IMG_DIR)) {
  const foreign = usesMusl()
    ? (name) => !name.includes("musl")
    : (name) => name.includes("musl");
  for (const name of readdirSync(IMG_DIR).filter(foreign)) {
    rmSync(join(IMG_DIR, name), { recursive: true, force: true });
    console.log(`pruned foreign sharp variant: @img/${name}`);
  }
}
