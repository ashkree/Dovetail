// The project and slice commands of ADR 0021, in the core's terms.

import { describe, expect, it } from 'vitest';
import { coreSnapshot } from './fixtures';
import { translate } from './translate';

const core = coreSnapshot(0, 'shop');

describe('translate', () => {
  it('creates a project with an optional account and first repo', () => {
    expect(translate({ type: 'create_project', name: 'Shop', account: ' ', repo_path: '~/code/shop-api ' }, core).core)
      .toEqual([{ create_project: { name: 'Shop', account: null, repo: '~/code/shop-api' } }]);
  });

  it('adds a repo by its folder, named or not', () => {
    expect(translate({ type: 'add_repo', project_id: 'shop', path: '/code/web', name: '' }, core).core)
      .toEqual([{ add_repo: { project: 'shop', path: '/code/web', name: null } }]);
  });

  it('starts a slice in a named repo on a task', () => {
    expect(translate({ type: 'new_slice', name: ' ui ', brief: 'go', task_id: '3', repo: 'shop-web' }, core).core)
      .toEqual([{ new_slice: { task: 3, name: 'ui', brief: 'go', repo: 'shop-web' } }]);
  });
});
