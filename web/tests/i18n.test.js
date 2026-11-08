import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolveLanguage, translate, chinese, applyLanguage } from '../src/i18n.js';

test('explicit language survives different system language and user data stays intact', () => {
  assert.equal(resolveLanguage(null, 'zh-TW'), 'zh-CN');
  assert.equal(resolveLanguage('en', 'zh-CN'), 'en');
  assert.equal(resolveLanguage('zh-CN', 'en'), 'zh-CN');
  assert.equal(translate('zh-CN', 'Connection'), '连接');
  assert.equal(translate('en', 'Connection'), 'Connection');
  assert.equal(translate('zh-CN', 'My host 中文.txt'), 'My host 中文.txt');
});
test('all static interface and accessibility keys have translations', () => {
  const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
  for (const [, key] of html.matchAll(/data-i18n(?:-title|-aria-label)?="([^"]+)"/g)) assert.ok(chinese[key], key);
});
test('switch updates existing status and accessibility labels in both directions', () => {
  const status = { dataset: { i18n: 'Connecting…' }, textContent: '' };
  const label = { getAttribute: () => 'Close connection panel', setAttribute: (_, value) => { label.value = value; } };
  const root = { documentElement: {}, querySelectorAll: selector => selector === '[data-i18n]' ? [status] : selector === '[data-i18n-aria-label]' ? [label] : [] };
  applyLanguage(root, 'zh-CN');
  assert.equal(status.textContent, '正在连接…'); assert.equal(label.value, '关闭连接面板');
  applyLanguage(root, 'en');
  assert.equal(status.textContent, 'Connecting…'); assert.equal(root.documentElement.lang, 'en');
});
