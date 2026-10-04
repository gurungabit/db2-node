import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

const packageRoot = fileURLToPath(new URL('../../crates/db2-napi/', import.meta.url));
const nodeTypeRoot = fileURLToPath(new URL('./node_modules/@types/', import.meta.url));

test('published declarations compile in isolated downstream consumers', async t => {
  const temporary = mkdtempSync(path.join(tmpdir(), 'db2-node-type-consumer-'));
  try {
    const installed = path.join(temporary, 'node_modules', 'db2-node');
    mkdirSync(installed, { recursive: true });
    // Copy only declarations actually included in the npm package, rather than
    // importing the checkout through a symlink that can find its ambient types.
    const [packed] = JSON.parse(execFileSync('npm', ['pack', '--dry-run', '--ignore-scripts', '--json'], {
      cwd: packageRoot, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'],
    }));
    for (const { path: relative } of packed.files) {
      if (relative === 'package.json' || relative.endsWith('.d.ts')) {
        const target = path.join(installed, relative);
        mkdirSync(path.dirname(target), { recursive: true });
        copyFileSync(path.join(packageRoot, relative), target);
      }
    }
    const manifest = JSON.parse(readFileSync(path.join(installed, 'package.json'), 'utf8'));
    assert.deepEqual(manifest.dependencies ?? {}, {}, 'public declarations must not require a new dependency');
    const emptyTypes = path.join(temporary, 'empty-types');
    mkdirSync(emptyTypes);

    const cases = [
      { fixture: 'client', target: ts.ScriptTarget.ES5, withNode: false },
      { fixture: 'client', target: ts.ScriptTarget.ES2022, withNode: false },
      { fixture: 'stream', target: ts.ScriptTarget.ES2022, withNode: false },
      { fixture: 'node-stream', target: ts.ScriptTarget.ES2022, withNode: true },
    ];
    for (const extension of ['cts', 'mts']) {
      for (const { fixture, target, withNode } of cases) {
        const label = `${extension} ${fixture} ${ts.ScriptTarget[target]} ${withNode ? 'with' : 'without'} Node typings`;
        await t.test(label, () => {
          const consumer = path.join(temporary, `${fixture}-${target}.${extension}`);
          copyFileSync(fileURLToPath(new URL(`./type-consumers/${fixture}.ts`, import.meta.url)), consumer);
          const program = ts.createProgram([consumer], {
            strict: true, noEmit: true, skipLibCheck: false, exactOptionalPropertyTypes: true,
            target,
            module: ts.ModuleKind.Node16, moduleResolution: ts.ModuleResolutionKind.Node16,
            lib: [target === ts.ScriptTarget.ES5 ? 'lib.es5.d.ts' : 'lib.es2022.d.ts'],
            types: withNode ? ['node'] : [], typeRoots: [withNode ? nodeTypeRoot : emptyTypes],
          });
          const diagnostics = ts.getPreEmitDiagnostics(program);
          assert.equal(diagnostics.length, 0, ts.formatDiagnostics(diagnostics, {
            getCurrentDirectory: () => temporary,
            getCanonicalFileName: name => name,
            getNewLine: () => '\n',
          }));
          const loadsNode = program.getSourceFiles().some(file => file.fileName.includes('/@types/node/'));
          assert.equal(loadsNode, withNode, 'ambient Node declarations must match the intended consumer environment');
        });
      }
    }
  } finally { rmSync(temporary, { recursive: true, force: true }); }
});
