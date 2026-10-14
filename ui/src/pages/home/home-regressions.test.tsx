import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import { cleanup, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { fakeBackend, renderWithClient } from '@/test/utils';
import { TOOLS } from '@/lib/tools';
import { probeGroups, type Row } from './model';
import i18n from '@/i18n';
const backend = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => backend.invoke(...args) }));
import { HomePage } from '../HomePage';
const cards = ['one', 'two'].map(id => ({ id, name: 'Shared', settingsConfig: { env: { ANTHROPIC_BASE_URL: 'https://relay.example/v1', ANTHROPIC_AUTH_TOKEN: `fake-${id}` } } }));
beforeEach(async () => { await i18n.changeLanguage('zh'); });
afterEach(cleanup);
it('probes an endpoint once while keeping same-named cards individually selectable', async () => {
  const fake = fakeBackend({
    get_overview: () => TOOLS.map(t => ({ tool: t.id, detected: true })),
    list_providers: args => ({ tool: args!.tool, kind: 'switch', current: 'one', providers: args!.tool === 'claude' ? cards : args!.tool === 'hermes' ? [{id:'hermes',name:'Shared',settingsConfig:{base_url:'https://relay.example/v1'}}] : [], inLive: [], paths: [] }),
    probe_latency: () => ({ ok: true, latencyMs: 123, status: 200 }),
  });
  backend.invoke = fake.invoke;
  renderWithClient(<HomePage mode="map" onNavigate={() => {}} />);
  expect(await screen.findAllByText('Shared')).toHaveLength(1);
  expect(screen.getAllByRole('button', { name: /Shared.*\([123]\/3\)/ })).toHaveLength(3);
  await userEvent.click(screen.getByRole('button', { name: '测连通' }));
  await waitFor(() => expect(fake.calls.filter(c => c.cmd === 'probe_latency')).toHaveLength(1));
  expect(await screen.findAllByText('123 ms')).toHaveLength(1);
});
it('does not combine different paths on the same host', () => {
  const row: Row = { tool: TOOLS[0], state: 'ready', active: [], others: [], data: { tool:'claude',kind:'switch',current:null,inLive:[],liveError:null,paths:[],providers: [cards[0], { ...cards[1],settingsConfig: {env:{ANTHROPIC_BASE_URL:'https://relay.example/v2'}} }] } };
  expect(probeGroups([row])).toHaveLength(2);
});
