import assert from "node:assert/strict";
import test from "node:test";

import { verifyNativeProductDependencyBoundary } from "./verify-native-product-dependency-boundary.mjs";

function metadata(extraPackage, extraDependencyKind = null) {
  const appId = "path+file:///repo/apps/desktop-mobile/src-tauri#lorepia-app@0.1.0";
  const coreId = "path+file:///repo/crates/lorepia-core#0.1.0";
  const packages = [
    {
      id: appId,
      name: "lorepia-app",
      manifest_path: "/repo/apps/desktop-mobile/src-tauri/Cargo.toml",
    },
    {
      id: coreId,
      name: "lorepia-core",
      manifest_path: "/repo/crates/lorepia-core/Cargo.toml",
    },
  ];
  const appDeps = [
    { pkg: coreId, dep_kinds: [{ kind: null, target: null }] },
  ];
  if (extraPackage) {
    packages.push(extraPackage);
    appDeps.push({
      pkg: extraPackage.id,
      dep_kinds: [{ kind: extraDependencyKind, target: null }],
    });
  }
  return {
    packages,
    resolve: {
      nodes: [
        { id: appId, deps: appDeps },
        { id: coreId, deps: [] },
        ...(extraPackage ? [{ id: extraPackage.id, deps: [] }] : []),
      ],
    },
  };
}

test("accepts the product closure without executable script runtimes", () => {
  assert.deepEqual(verifyNativeProductDependencyBoundary(metadata()), {
    product: "lorepia-app",
    packagesChecked: 2,
  });
});

test("rejects a transitive spike even if its package name looks harmless", () => {
  const spike = {
    id: "path+file:///repo/spikes/runner#harmless@0.1.0",
    name: "harmless",
    manifest_path: "/repo/spikes/runner/Cargo.toml",
  };
  assert.throws(
    () => verifyNativeProductDependencyBoundary(metadata(spike)),
    /spike package/,
  );
});

test("rejects a native QuickJS or Lua runtime but ignores dev-only tools", () => {
  const runtime = {
    id: "registry+https://example.invalid#index#rquickjs@1.0.0",
    name: "rquickjs",
    manifest_path: "/cargo/registry/rquickjs/Cargo.toml",
  };
  assert.throws(
    () => verifyNativeProductDependencyBoundary(metadata(runtime)),
    /executable runtime/,
  );
  assert.deepEqual(
    verifyNativeProductDependencyBoundary(metadata(runtime, "dev")),
    { product: "lorepia-app", packagesChecked: 2 },
  );
});

test("rejects runtime sys crates with library prefixes", () => {
  for (const name of ["libquickjs-sys", "quickjs-ng-sys", "liblua54-sys"]) {
    const runtime = {
      id: `registry+https://example.invalid#index#${name}@1.0.0`,
      name,
      manifest_path: `/cargo/registry/${name}/Cargo.toml`,
    };
    assert.throws(
      () => verifyNativeProductDependencyBoundary(metadata(runtime)),
      /executable runtime/,
      name,
    );
  }
});
