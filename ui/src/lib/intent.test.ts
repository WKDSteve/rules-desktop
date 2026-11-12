import { describe, expect, it } from 'vitest';
import { App } from './app.svelte';
import { intentProblems, MAX_BYTES, readIntent } from './intent';
import aiHandoff from './wkd-intent/ai.handoff.json';
import rulesNew from './wkd-intent/rules.new.json';
import rulesShow from './wkd-intent/rules.show.json';
import tasksNew from './wkd-intent/tasks.new.json';
import tasksShow from './wkd-intent/tasks.show.json';
import { FakeHub, testEnv } from './testhub';

const text = (doc: unknown): string => JSON.stringify(doc);
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const edited = (doc: object, change: (copy: Record<string, any>) => void): string => {
  const copy = JSON.parse(JSON.stringify(doc));
  change(copy);
  return JSON.stringify(copy);
};

describe('the check of a wkd-intent/1 (the golden examples of the contract)', () => {
  it('takes the two goldens of its own actions', () => {
    expect(intentProblems(text(rulesShow))).toEqual([]);
    expect(intentProblems(text(rulesNew))).toEqual([]);
  });

  it('refuses the actions of the other apps: this target did not offer them', () => {
    expect(intentProblems(text(tasksNew))).toEqual(['this target does not take tasks.new']);
    expect(intentProblems(text(tasksShow))).toEqual(['this target does not take tasks.show']);
    expect(intentProblems(text(aiHandoff))).toEqual(['this target does not take ai.handoff']);
  });

  it('refuses what is not one JSON object of this schema, with the reason', () => {
    expect(intentProblems('{')).toEqual(['not JSON in UTF-8']);
    expect(intentProblems('[]')).toEqual(['not a JSON object']);
    expect(intentProblems(edited(rulesShow, (d) => (d.schema = 'wkd-intent/2')))).toEqual([
      'schema "wkd-intent/2" is not wkd-intent/1',
    ]);
    expect(intentProblems(edited(rulesShow, (d) => (d.action = 'rules.delete')))).toEqual(['unknown action']);
    expect(intentProblems(edited(rulesShow, (d) => (d.mode = 'agent')))).toEqual(['rules.show only knows mode form']);
    expect(intentProblems(edited(rulesShow, (d) => (d.finding = rulesNew.finding)))).toEqual([
      'rules.show carries no finding',
    ]);
    expect(intentProblems(edited(rulesNew, (d) => (d.finding = null)))).toEqual(['rules.new needs a finding']);
    expect(intentProblems(edited(rulesNew, (d) => (d.target = { id: 'X-1' })))).toEqual(['rules.new has no target']);
  });

  it('measures the size in bytes before parsing', () => {
    expect(intentProblems(' '.repeat(MAX_BYTES + 1))).toEqual([`larger than ${MAX_BYTES} bytes`]);
    // two bytes per character: the limit is bytes, not characters
    expect(intentProblems('é'.repeat(MAX_BYTES / 2 + 1))).toEqual([`larger than ${MAX_BYTES} bytes`]);
  });

  it('refuses a path that leaves the repository, in the id, the file and the repro', () => {
    for (const id of ['/etc/passwd', '\\x', 'C:\\x', 'C:/x', '\\\\host\\x', '~/x', '$HOME/x', '../x', 'a/../x']) {
      expect(intentProblems(edited(rulesShow, (d) => (d.target.id = id))), id).toEqual([
        'target.id leaves the repository',
      ]);
    }
    expect(intentProblems(edited(rulesNew, (d) => (d.finding.where.file = '/abs/file.lua')))).toEqual([
      'finding.where.file leaves the repository',
    ]);
    expect(intentProblems(edited(rulesNew, (d) => (d.finding.repro = 'run --out=/etc/x')))).toEqual([
      'finding.repro leaves the repository',
    ]);
    expect(intentProblems(edited(rulesShow, (d) => (d.target.id = 'LUA-12')))).toEqual([]);
  });

  it('applies the limits of the contract and refuses control characters', () => {
    expect(intentProblems(edited(rulesNew, (d) => (d.finding.title = 'x'.repeat(121))))).toEqual([
      'finding.title is longer than 120',
    ]);
    expect(intentProblems(edited(rulesShow, (d) => (d.target.id = 'x'.repeat(301))))).toEqual([
      'target.id is longer than 300',
    ]);
    expect(intentProblems(edited(rulesNew, (d) => (d.finding.excerpt = 'a\u0007b')))).toEqual([
      'a field holds a control character',
    ]);
    expect(intentProblems(edited(rulesNew, (d) => (d.finding.excerpt = 'a\tb\nc')))).toEqual([]);
    expect(intentProblems(edited(rulesNew, (d) => (d.finding.class = 'secret')))).toEqual([
      'finding.class is not public or internal',
    ]);
    expect(intentProblems(edited(rulesNew, (d) => (d.finding.run = { url: 'http://evil.example/x' })))).toEqual([
      'finding.run.url does not start with https://github.com/',
    ]);
  });
});

describe('what an accepted intent asks for', () => {
  it('reads rules.show as the rule to show and rules.new as the start of the form', () => {
    expect(readIntent(text(rulesShow))).toEqual({ ok: true, intent: { action: 'rules.show', id: 'LUA-12' } });
    expect(readIntent(text(rulesNew))).toEqual({
      ok: true,
      intent: {
        action: 'rules.new',
        title: 'the same check fails in three repositories',
        text: 'a global is set at load time\n\nlua/example/init.lua:3',
      },
    });
    expect(readIntent('nope')).toEqual({ ok: false, problems: ['not JSON in UTF-8'] });
  });
});

describe('the app takes an intent', () => {
  function ready(): App {
    const a = new App(testEnv(new FakeHub()).env);
    a.phase = 'ready';
    return a;
  }

  it('opens the form filled in for rules.new, once', () => {
    const a = ready();
    expect(a.takeIntent(text(rulesNew))).toEqual([]);
    expect(a.newRuleOpen).toBe(true);
    expect(a.newRulePrefill?.title).toBe('the same check fails in three repositories');
    a.closeNewRule();
    a.openNewRule();
    expect(a.newRulePrefill).toBeNull();
  });

  it('opens the rules view for rules.show, and writes and opens nothing for a refused intent', () => {
    const a = ready();
    expect(a.takeIntent(text(rulesShow))).toEqual([]);
    expect(a.view).toBe('rules');
    const b = ready();
    expect(b.takeIntent(text(tasksNew))).toEqual(['this target does not take tasks.new']);
    expect(b.newRuleOpen).toBe(false);
  });

  it('refuses while the hub is not connected', () => {
    const a = new App(testEnv(new FakeHub()).env);
    expect(a.takeIntent(text(rulesShow))).toEqual(['the hub is not connected yet']);
  });
});
