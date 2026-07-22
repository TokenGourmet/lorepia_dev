import { readFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';

function cargoPurl(name, version) {
  return `pkg:cargo/${encodeURIComponent(name)}@${encodeURIComponent(version)}`;
}

export function cargoMetadataToCycloneDx(metadata) {
  if (
    typeof metadata !== 'object' ||
    metadata === null ||
    !Array.isArray(metadata.packages) ||
    typeof metadata.resolve !== 'object' ||
    metadata.resolve === null ||
    !Array.isArray(metadata.resolve.nodes)
  ) {
    throw new Error('INVALID_CARGO_METADATA');
  }

  const packagesById = new Map();
  for (const pkg of metadata.packages) {
    if (
      typeof pkg !== 'object' ||
      pkg === null ||
      typeof pkg.id !== 'string' ||
      typeof pkg.name !== 'string' ||
      typeof pkg.version !== 'string'
    ) {
      throw new Error('INVALID_CARGO_PACKAGE');
    }
    if (packagesById.has(pkg.id)) throw new Error('DUPLICATE_CARGO_PACKAGE');
    packagesById.set(pkg.id, pkg);
  }

  const components = [...packagesById.values()]
    .map((pkg) => {
      const purl = cargoPurl(pkg.name, pkg.version);
      const component = {
        type: metadata.workspace_members?.includes(pkg.id) ? 'application' : 'library',
        'bom-ref': purl,
        name: pkg.name,
        version: pkg.version,
        purl,
      };
      if (typeof pkg.license === 'string' && pkg.license.length > 0) {
        component.licenses = [{ expression: pkg.license }];
      }
      if (typeof pkg.repository === 'string' && pkg.repository.length > 0) {
        component.externalReferences = [{ type: 'vcs', url: pkg.repository }];
      }
      return component;
    })
    .sort((left, right) => left['bom-ref'].localeCompare(right['bom-ref']));

  const dependencies = metadata.resolve.nodes
    .map((node) => {
      const pkg = packagesById.get(node.id);
      if (!pkg || !Array.isArray(node.dependencies)) {
        throw new Error('INVALID_CARGO_RESOLVE_NODE');
      }
      const dependsOn = node.dependencies.map((dependencyId) => {
        const dependency = packagesById.get(dependencyId);
        if (!dependency) throw new Error('UNKNOWN_CARGO_DEPENDENCY');
        return cargoPurl(dependency.name, dependency.version);
      });
      return {
        ref: cargoPurl(pkg.name, pkg.version),
        dependsOn: [...new Set(dependsOn)].sort(),
      };
    })
    .sort((left, right) => left.ref.localeCompare(right.ref));

  return {
    bomFormat: 'CycloneDX',
    specVersion: '1.5',
    version: 1,
    metadata: {
      component: {
        type: 'application',
        'bom-ref': 'pkg:generic/lorepia@workspace',
        name: 'LorePia',
        version: 'workspace',
      },
    },
    components,
    dependencies,
  };
}

async function main() {
  const input = process.argv[2]
    ? await readFile(process.argv[2], 'utf8')
    : await new Promise((resolve, reject) => {
        let value = '';
        process.stdin.setEncoding('utf8');
        process.stdin.on('data', (chunk) => {
          value += chunk;
        });
        process.stdin.on('end', () => resolve(value));
        process.stdin.on('error', reject);
      });
  const result = cargoMetadataToCycloneDx(JSON.parse(input));
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
