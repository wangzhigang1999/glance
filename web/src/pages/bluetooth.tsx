import { useState } from "react";
import { Bluetooth, Power, Plus, Minus, RefreshCw } from "lucide-react";
import { usePoll } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Notice } from "@/components/common";

type State = {
  peers: { name: string; address: string; enabled: boolean; state: string; error: string }[];
  devices: { address: string; name: string; rssi: number; age_s: number; supported: boolean }[];
  bindings: { key: string; action: string }[];
  reports: { seq: number; key: string; hex: string; uptime_ms: number }[];
  learning: string | null;
  lamp_level: number | null;
  lamp_battery_raw: number | null;
  lamp_battery_age_s: number | null;
  last_action: string;
  error: string;
  dropped_events: number;
  lamp_ack_ms: number | null;
  gesture_ms: number | null;
  lamp_timeouts: number;
};
const actions = [{ id: "toggle", label: "开关灯", Icon: Power }, { id: "up", label: "调亮", Icon: Plus }, { id: "down", label: "调暗", Icon: Minus }];

export function BluetoothPage() {
  const state = usePoll<State>("/api/bluetooth", 2000);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function command(body: object) {
    setBusy(true); setError("");
    try {
      const r = await fetch("/api/bluetooth", { method: "POST", headers: { "Content-Type": "application/json", "X-Admin-Key": key }, body: JSON.stringify(body), signal: AbortSignal.timeout(10000) });
      if (!r.ok) throw Error(r.status === 403 ? "管理口令不正确或请求来源不允许" : "操作未被接受，请稍后重试");
      state.refresh();
    } catch (e) { setError(e instanceof Error ? e.message : "操作失败"); }
    finally { setBusy(false); }
  }
  const s = state.data;
  const disabled = busy || !key || !!state.error;
  return <div className="space-y-5">
    <section className="panel space-y-3 p-5">
      <div className="flex items-center justify-between"><div className="flex items-center gap-2 font-medium"><Bluetooth size={19} />蓝牙管理</div><Button variant="outline" size="icon" aria-label="刷新" onClick={state.refresh}><RefreshCw size={16} /></Button></div>
      <p className="caption">由板子连接翻页器和灯，电脑无需常开。体重与温湿度继续通过广播采集。</p>
      <label className="block max-w-sm space-y-2 text-sm">管理口令<Input type="password" autoComplete="off" value={key} onChange={e => setKey(e.target.value)} placeholder="沿用设备管理口令" /></label>
      <Notice error>{error || state.error || s?.error}</Notice>
      {s && <p className="caption">最近手势识别：{s.gesture_ms === null ? "尚无记录" : `${s.gesture_ms} 毫秒`} · 灯控确认：{s.lamp_ack_ms === null ? "尚无记录" : `${s.lamp_ack_ms} 毫秒`} · 本次启动确认超时：{s.lamp_timeouts} 次</p>}
    </section>
    <div className="grid gap-4 md:grid-cols-2">{s?.peers.map((p, i) => <section key={p.address} className="panel space-y-4 p-5">
      <div className="flex items-center justify-between gap-3"><h2 className="font-medium">{p.name}</h2><span className="text-sm text-muted-foreground">{p.state}</span></div>
      <p className="caption font-mono">{p.address}</p><Notice error>{p.error}</Notice>
      {i === 1 && <p className="text-sm">灯电量：{p.state === "已就绪" && s.lamp_battery_raw !== null && s.lamp_battery_age_s !== null && s.lamp_battery_age_s < 600 ? `${s.lamp_battery_raw}%（设备估值，${Math.floor(s.lamp_battery_age_s / 60)} 分钟前）` : "等待新的读数"}</p>}
      <Button disabled={disabled} variant={p.enabled ? "outline" : "default"} onClick={() => command({ op: "enable", device: i, enabled: !p.enabled })}>{p.enabled ? "停用连接" : "连接设备"}</Button>
      {i === 1 && <><p className="text-sm">已确认亮度：{s.lamp_level === null ? "未知" : `${s.lamp_level} 档`}</p><div className="flex flex-wrap gap-2">{[{id:"on",label:"开灯"},{id:"off",label:"关灯"},...actions.slice(1)].map(a=><Button key={a.id} variant="outline" disabled={disabled || p.state !== "已就绪"} onClick={()=>command({op:"lamp",action:a.id})}>{a.label}</Button>)}</div><p className="caption">{s.last_action || "每次调节 10 档。手动改变灯的状态不会同步。"}</p></>}
    </section>)}</div>
    <section className="panel space-y-4 p-5"><h2 className="font-medium">按键绑定</h2><p className="caption">点击学习，再短按翻页器上的目标按钮。等待 20 秒，绑定后自动保存。</p>
      <div className="grid gap-3 sm:grid-cols-3">{actions.map(({id,label,Icon})=><div key={id} className="rounded-xl border p-4 space-y-3"><div className="flex items-center gap-2"><Icon size={17}/>{label}</div><p className="caption break-all">{s?.bindings.find(b=>b.action===id) ? "已绑定" : "未绑定"}</p><Button variant="outline" disabled={disabled || s?.peers[0]?.state!=="已就绪" || !!s?.learning} onClick={()=>command({op:"learn",action:id})}>学习按键</Button></div>)}</div>
      {s?.learning && <div className="flex items-center gap-3 text-sm">等待按键：{actions.find(a=>a.id===s.learning)?.label}<Button variant="outline" disabled={disabled} onClick={()=>command({op:"cancel_learn"})}>取消</Button></div>}
      {!!s?.bindings.length && <Button variant="outline" disabled={disabled || !!s?.learning} onClick={()=>command({op:"clear_bindings"})}>清除绑定</Button>}
    </section>
    <section className="panel space-y-3 p-5"><h2 className="font-medium">最近按键报告</h2><p className="caption">最新 20 条，用于确认翻页器是否正在发送。释放按键的零值报告也会保留。</p>{!s?.reports.length ? <p className="text-sm text-muted-foreground">尚未收到报告</p> : <div className="max-h-64 overflow-auto space-y-2">{[...s.reports].reverse().map(r=><div key={r.seq} className="flex gap-3 border-b py-2 text-sm"><span className="text-muted-foreground">#{r.seq}</span><code className="break-all">{r.key}</code></div>)}</div>}{!!s?.dropped_events && <Notice error>队列曾丢弃 {s.dropped_events} 条事件，请检查负载。</Notice>}</section>
    <section className="panel space-y-3 p-5"><h2 className="font-medium">附近设备</h2><p className="caption">持续扫描，最多显示 24 台。当前支持管理已确认的翻页器与灯，其他设备仅显示广播。</p><div className="divide-y">{s?.devices.filter(d=>d.age_s<120).sort((a,b)=>b.rssi-a.rssi).map(d=><div key={d.address} className="flex justify-between gap-3 py-3"><div className="min-w-0"><p className="text-sm truncate">{d.name || "未命名设备"}</p><p className="caption font-mono">{d.address}</p></div><div className="text-right text-sm text-muted-foreground">{d.rssi} dBm<p className="caption">{d.age_s} 秒前</p></div></div>)}</div></section>
  </div>;
}
