import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

test('the native Vim runtime detects and highlights Rastok without user configuration', () => {
  const scratch = mkdtempSync(join(tmpdir(), 'rastok-vim-'));
  try {
    const log = join(scratch, 'result.txt');
    const result = spawnSync(process.env.RASTOK_VIM || 'vim', [
      '-Nu', 'NONE', '-n', '-i', 'NONE', '-es', '-S',
      fileURLToPath(new URL('./test.vim', import.meta.url)),
    ], {
      cwd: scratch,
      encoding: 'utf8',
      timeout: 30_000,
      env: { ...process.env, RASTOK_EDITOR_TEST_LOG: log },
    });
    const detail = existsSync(log) ? readFileSync(log, 'utf8') : result.stderr;
    assert.ifError(result.error);
    assert.equal(result.status, 0, detail);
    console.log(detail.trim());
  } finally {
    rmSync(scratch, { recursive: true });
  }
});
