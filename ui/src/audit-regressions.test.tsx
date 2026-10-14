// Audit-only reproductions. Tauri commands are mocked; no real config or account is touched.
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import { cleanup, screen, within, fireEvent, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { fakeBackend, renderWithClient } from '@/test/utils';
import i18n from '@/i18n';
import { modelOf } from '@/lib/cards';
import { TOOLS } from '@/lib/tools';
import type { Provider } from '@/lib/api';

const backend = vi.hoisted(() => ({invoke:vi.fn()}));
vi.mock('@tauri-apps/api/core',()=>({invoke:(...args:unknown[])=>backend.invoke(...args)}));
vi.mock('@tauri-apps/api/event',()=>({listen:vi.fn(async()=>()=>{})}));
vi.mock('sonner',()=>({toast:{success:vi.fn(),error:vi.fn(),info:vi.fn(),warning:vi.fn()}}));
import App from '@/App';
import { StatsPage } from '@/pages/StatsPage';
import { ProviderEditor } from '@/components/ProviderEditor';

const a: Provider = {id:'a',name:'Audit A',settingsConfig:{env:{ANTHROPIC_BASE_URL:'https://a.example.invalid',ANTHROPIC_AUTH_TOKEN:'fake'}}};
const b: Provider = {id:'b',name:'Audit B',settingsConfig:{env:{ANTHROPIC_BASE_URL:'https://b.example.invalid',ANTHROPIC_AUTH_TOKEN:'fake'}}};
const settings=()=>({app:{language:'zh',theme:'dark',dirOverrides:{},autostart:false,backgroundRefreshHours:168,closeToTray:true},keepCodexLogin:false,dataDir:'/audit/.leanflip',home:'/audit',livePaths:{},dirs:{codexDir:'/audit/.codex'},ccswitchDir:'/audit/.cc-switch'});
function start(extra:Record<string,(args:any)=>any>={}) {
  let current='a';
  const fake=fakeBackend({
    get_settings:settings,get_recovery_notices:()=>[],
    get_overview:()=>TOOLS.map(t=>({tool:t.id,kind:t.kind,paths:[],detected:true,cardCount:t.id==='claude'?2:0,current:t.id==='claude'?current:null,currentName:t.id==='claude'?`Audit ${current.toUpperCase()}`:null,inLive:[],liveReadable:true})),
    list_providers:(args)=>({tool:args!.tool,kind:TOOLS.find(t=>t.id===args!.tool)!.kind,providers:args!.tool==='claude'?[a,b]:[],current:args!.tool==='claude'?current:null,inLive:[],liveError:null,paths:[]}),
    codex_accounts:()=>[],codex_logins:()=>[],ignored_fields:()=>[],
    switch_provider:(args)=>{current=String(args!.id);return {warnings:[],changed:[]}},
    ...extra,
  });backend.invoke=fake.invoke;return fake;
}
beforeEach(async()=>{vi.clearAllMocks();window.history.replaceState(null,'','/#tool=claude');await i18n.changeLanguage('zh');});
afterEach(()=>{cleanup();window.history.replaceState(null,'','/');});

it('Regression: command palette must wait for settings navigation consent before switching',async()=>{
  const fake=start();renderWithClient(<App/>);
  await userEvent.click(screen.getByRole('link',{name:'设置'}));
  await userEvent.type(await screen.findByLabelText('Codex (~/.codex)'),'/audit/draft');
  await userEvent.click(screen.getByRole('button',{name:'打开命令面板'}));
  await userEvent.click(await screen.findByRole('option',{name:/Claude Code › Audit B/}));
  const dialog=await screen.findByRole('alertdialog');
  await userEvent.click(within(dialog).getByRole('button',{name:'取消'}));
  expect(window.location.hash).toBe('#settings');
  expect(fake.calls.filter(c=>c.cmd==='switch_provider')).toHaveLength(0);
});

it('Regression: command palette navigation should protect a dirty provider editor',async()=>{
  start();renderWithClient(<App/>);
  await userEvent.click(await screen.findByRole('button',{name:'编辑 Audit A'}));
  const dialog=await screen.findByRole('dialog');
  const name=within(dialog).getByLabelText('名称');
  await userEvent.clear(name);await userEvent.type(name,'Unsaved audit draft');
  fireEvent.keyDown(window,{key:'k',metaKey:true});
  await userEvent.click(await screen.findByRole('option',{name:'Gemini CLI 打开'}));
  await waitFor(()=>expect(screen.queryByRole('dialog',{name:'命令面板'})).not.toBeInTheDocument());
  expect(screen.getByDisplayValue('Unsaved audit draft')).toBeInTheDocument();
});

it('Regression: model display must parse valid single quoted TOML strings',()=>{
  expect(modelOf('codex',{id:'audit',name:'audit',settingsConfig:{config:"model = 'audit-model'\n"}})).toBe('audit-model');
});

it('Regression: model display must not use a nested profile model as the active root model',()=>{
  expect(modelOf('codex',{id:'audit',name:'audit',settingsConfig:{config:'[profiles.unused]\nmodel = "wrong-model"\n'}})).toBeUndefined();
});

it('Regression: timeline should not offer switching to a deleted provider',async()=>{
  const now=new Date(2026,9,14,12).getTime();
  const fake=start({usage_report:()=>({rows:[],events:[{tool:'claude',provider:'deleted',name:'Deleted audit',ts:now-7200000,enabled:true},{tool:'claude',provider:'a',name:'Audit A',ts:now-3600000,enabled:true}],sources:[],scannedAt:now})});
  renderWithClient(<StatsPage/>);
  const deleted=await screen.findByRole('button',{name:/Claude Code：Deleted audit/});
  await userEvent.click(deleted);
  expect(fake.calls.filter(c=>c.cmd==='switch_provider')).toHaveLength(0);
});

it('Regression: timeline must not divide one hour provider usage by 24',async()=>{
  const now=new Date(2026,9,14,12).getTime();
  start({usage_report:()=>({rows:[{day:'2026-10-14',tool:'claude',provider:'a',model:'audit',attribution:'time',input:24000,cacheRead:0,cacheWrite:0,output:0,requests:1}],events:[{tool:'claude',provider:'a',name:'Audit A',ts:now-3600000,enabled:true}],sources:[],scannedAt:now})});
  renderWithClient(<StatsPage/>);
  const segment=await screen.findByRole('button',{name:/Claude Code：Audit A/});
  fireEvent.focus(segment);
  const tip=await screen.findByRole('tooltip');
  expect(tip.textContent).toContain('无法准确计算此时段用量');
  expect(tip.textContent).not.toContain('1,000');
});

it('Regression: login reference is validated with the provider in one save',async()=>{
  const fake=start({add_provider:()=>{throw new Error('Referenced login was not found')}});
  const template:Provider={id:'official',name:'Official',category:'official',settingsConfig:{auth:{},config:''},meta:{codexLoginRef:'local-audit'}};
  renderWithClient(<ProviderEditor tool="codex" open onOpenChange={()=>{}} template={template} existingIds={['official']} logins={[{identity:'local-audit',email:'audit@example.invalid',workspace:null,plan:'pro',live:true}]} />);
  await userEvent.click(await screen.findByRole('button',{name:'保存'}));
  expect(await screen.findByText(/Referenced login was not found/)).toBeInTheDocument();
  const save=fake.calls.find(c=>c.cmd==='add_provider')!;
  expect((save.args!.provider as Provider).meta?.codexLoginRef).toBe('local-audit');
  expect(fake.calls.filter(c=>c.cmd==='codex_set_login_ref')).toHaveLength(0);
});

it('Regression: partial previous periods do not show growth percentages', async () => {
  const row = (day: string, input: number) => ({ day, input, tool:'claude', provider:'a', model:'m', attribution:'time', cacheRead:0,cacheWrite:0,output:0,requests:1 });
  start({ usage_report:()=>({rows:[row('2026-10-14',1000),row('2026-09-14',10)],events:[],sources:[],scannedAt:new Date(2026,9,14,12).getTime(),coverageStart:{claude:'2026-09-14'}}) });
  renderWithClient(<StatsPage/>);
  expect(await screen.findAllByText('前 30 天数据不足，暂不比较')).toHaveLength(3);
  expect(document.querySelector('.kpi-delta')).not.toHaveTextContent('%');
});

it('Regression: a full previous period retains the comparison and names ambiguous official usage', async () => {
  const row = (day: string, input: number) => ({ day, input, tool:'codex', provider:'codex-route:openai', model:'m', attribution:'route', cacheRead:0,cacheWrite:0,output:0,requests:1 });
  start({ usage_report:()=>({rows:[row('2026-10-14',200),row('2026-09-14',100)],events:[],sources:[],scannedAt:new Date(2026,9,14,12).getTime(),coverageStart:{codex:'2026-08-01'}}) });
  renderWithClient(<StatsPage/>);
  expect(await screen.findByText('↑ 100%，对比前 30 天')).toBeInTheDocument();
  expect(screen.getByText('官方 OpenAI（未区分卡片）')).toBeInTheDocument();
  expect(screen.getByText('记录自带路由')).toBeInTheDocument();
});
