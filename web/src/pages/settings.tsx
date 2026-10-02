import { useEffect, useState } from "react";
import { Save, RefreshCw, Wifi, Plus, Github } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Section, Field, Loading, Notice, Confirm } from "@/components/common";
import { api, type Config } from "@/lib/api";
import { configPatch } from "@/lib/policy.mjs";
type WifiList = { slots: { ssid: string }[]; max: number };
export function SettingsPage({ onDirty }: { onDirty: (v: boolean) => void }) {
  const [original, setOriginal] = useState<Config | null>(null),
    [form, setForm] = useState<Config>({});
  const [wifi, setWifi] = useState<WifiList | null>(null),
    [wifiError, setWifiError] = useState("");
  const [message, setMessage] = useState(""),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [adding, setAdding] = useState(false);
  const [ssid, setSsid] = useState(""),
    [password, setPassword] = useState("");
  async function load() {
    setError("");
    try {
      const c = await api<Config>("/api/config");
      setOriginal(c);
      setForm(c);
    } catch (e) {
      setError((e as Error).message);
    }
  }
  async function loadWifi() {
    try {
      setWifi(await api<WifiList>("/api/wifi"));
      setWifiError("");
    } catch (e) {
      setWifiError((e as Error).message);
    }
  }
  useEffect(() => {
    void load();
    void loadWifi();
  }, []);
  const changed = original
    ? JSON.stringify(configPatchSafe(original, form)) !== "{}"
    : false;
  useEffect(() => {
    onDirty(changed);
    return () => onDirty(false);
  }, [changed, onDirty]);
  useEffect(() => {
    const warn = (e: BeforeUnloadEvent) => {
      if (changed) {
        e.preventDefault();
        e.returnValue = "";
      }
    };
    addEventListener("beforeunload", warn);
    return () => removeEventListener("beforeunload", warn);
  }, [changed]);
  function set(key: string, value: string | number | boolean) {
    setForm((f) => ({ ...f, [key]: value }));
    setMessage("");
  }
  function field(
    key: string,
    label: string,
    min: number,
    max: number,
    step = 1,
    hint?: string,
  ) {
    return (
      <Field
        key={key}
        label={label}
        type="number"
        required
        min={min}
        max={max}
        step={step}
        value={String(form[key] ?? "")}
        onChange={(e) => set(key, e.target.value)}
        hint={hint}
      />
    );
  }
  async function save(e: React.FormEvent) {
    e.preventDefault();
    if (!original) return;
    setBusy(true);
    setError("");
    try {
      const patch = configPatch(original, form);
      const c = await api<Config>("/api/config", patch);
      setOriginal(c);
      setForm(c);
      setMessage("设置已保存，将在下一次刷新时生效。");
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  if (!original)
    return (
      <>
        <Notice error>{error}</Notice>
        {error ? (
          <Button variant="outline" onClick={load}>
            重新读取设置
          </Button>
        ) : (
          <Loading />
        )}
      </>
    );
  return (
    <div className="space-y-4 sm:space-y-6">
      <form onSubmit={save} className="space-y-4 sm:space-y-6">
        <div className="grid items-start gap-4 sm:gap-6 xl:grid-cols-2">
          <Section title="屏幕与刷新">
            <div className="space-y-4 sm:space-y-6">
              <div className="flex items-center justify-between gap-4 border-b pb-5">
                <div>
                  <label htmlFor="auto-rotate" className="text-sm font-medium">
                    自动轮播
                  </label>
                </div>
                <Switch
                  id="auto-rotate"
                  checked={!!form.auto_rotate}
                  onCheckedChange={(v) => set("auto_rotate", v)}
                />
              </div>
              <div className="grid gap-5 sm:grid-cols-2">
                {field("auto_rotate_s", "轮播间隔 / 秒", 3, 3600)}
                {field("sensor_refresh_s", "传感器刷新 / 秒", 1, 3600)}
                {field(
                  "splash_flash",
                  "开机清屏次数",
                  0,
                  64,
                  1,
                  "下次启动时生效",
                )}
                {field(
                  "tz_off_s",
                  "时区偏移 / 秒",
                  -50400,
                  50400,
                  1,
                  "北京时间为 28800",
                )}
              </div>
              <p className="caption border-t pt-4">
                短按 BOOT 键翻转实体屏幕。
              </p>
            </div>
          </Section>
          <Section title="GitHub">
            <div className="space-y-5">
              <Field
                label="用户名"
                value={String(form.gh_user || "")}
                maxLength={39}
                pattern="[A-Za-z0-9\-]*"
                autoComplete="off"
                onChange={(e) => set("gh_user", e.target.value)}
              />
              <Field
                label="访问令牌"
                type="password"
                autoComplete="off"
                value={String(form.gh_token || "")}
                maxLength={255}
                onChange={(e) => set("gh_token", e.target.value)}
                hint="已保存的令牌以 *** 显示。留空并保存会清除令牌。"
              />
              <Button
                type="button"
                variant="outline"
                disabled={busy}
                onClick={async () => {
                  setBusy(true);
                  setError("");
                  try {
                    const token = String(form.gh_token || "");
                    const j = await api<{ login: string }>(
                      "/api/whoami",
                      token && !token.startsWith("***") ? { token } : {},
                    );
                    set("gh_user", j.login);
                    setMessage("已识别用户名，保存后生效。");
                  } catch (e) {
                    setError((e as Error).message);
                  } finally {
                    setBusy(false);
                  }
                }}
              >
                <Github />
                识别用户名
              </Button>
              <div className="grid gap-5 sm:grid-cols-2">
                {field("gh_refresh_s", "正常刷新 / 秒", 30, 86400)}
                {field("gh_err_s", "失败重试 / 秒", 30, 86400)}
              </div>
            </div>
          </Section>
          <Section title="环境校正">
            <div className="grid gap-5 sm:grid-cols-2">
              {field("temp_off_c", "温度偏移 / °C", -20, 20, 0.1)}
              {field("humid_off_pct", "湿度偏移 / %", -50, 50, 0.1)}
            </div>
          </Section>
          <div className="space-y-4 self-end">
            <Notice error>{error}</Notice>
            <Notice>{message}</Notice>
            <div className="flex flex-wrap gap-3">
              <Button type="submit" disabled={busy || !changed}>
                <Save />
                {busy ? "处理中…" : changed ? "保存设置" : "已保存"}
              </Button>
              <Button
                type="button"
                variant="outline"
                disabled={busy}
                onClick={() => {
                  setForm(original);
                  setError("");
                  setMessage("已撤销未保存的更改。");
                }}
              >
                <RefreshCw />
                撤销更改
              </Button>
            </div>
          </div>
        </div>
      </form>
      <Section
        title="Wi-Fi 网络"
        action={
          <span className="caption">
            {wifi ? `${wifi.slots.length} / ${wifi.max} 个网络` : ""}
          </span>
        }
      >
        <Notice error>{wifiError}</Notice>
        <div className="grid gap-8 xl:grid-cols-2">
          <div>
            {wifi?.slots.length === 0 && (
              <p className="caption py-5">还没有保存的网络。</p>
            )}
            {wifi?.slots.map((network, i) => (
              <div
                key={network.ssid}
                className="flex items-center gap-3 border-b py-4 first:pt-0 last:border-0"
              >
                <span className="rounded-lg bg-muted p-3 text-primary">
                  <Wifi size={18} />
                </span>
                <div className="min-w-0 flex-1">
                  <p className="break-all text-sm font-medium">
                    {network.ssid}
                  </p>
                  <p className="caption">{i === 0 ? "最近使用" : "已保存"}</p>
                </div>
                <Confirm
                  label="移除"
                  title="移除已保存网络？"
                  description={`移除 ${network.ssid} 后，设备下次不会自动连接这个网络。`}
                  action={async () => {
                    await api("/api/wifi/remove", { ssid: network.ssid });
                    await loadWifi();
                  }}
                />
              </div>
            ))}
          </div>
          <form
            className="space-y-4"
            onSubmit={async (e) => {
              e.preventDefault();
              setAdding(true);
              setWifiError("");
              try {
                await api("/api/wifi", { ssid: ssid.trim(), password });
                setSsid("");
                setPassword("");
                await loadWifi();
              } catch (e) {
                setWifiError((e as Error).message);
              } finally {
                setAdding(false);
              }
            }}
          >
            <Field
              label="网络名称"
              value={ssid}
              required
              maxLength={32}
              autoComplete="off"
              onChange={(e) => setSsid(e.target.value)}
            />
            <Field
              label="网络密码"
              type="password"
              value={password}
              maxLength={64}
              autoComplete="new-password"
              onChange={(e) => setPassword(e.target.value)}
            />
            <Button
              type="submit"
              variant="outline"
              disabled={adding || !ssid.trim()}
            >
              <Plus />
              {adding ? "保存中…" : "保存网络"}
            </Button>
          </form>
        </div>
      </Section>
      <section className="flex flex-wrap items-center justify-between gap-5 rounded-xl border px-4 py-4 sm:px-6 sm:py-5">
        <div>
          <h2 className="section-title">设备维护</h2>
        </div>
        <div className="flex flex-wrap gap-3">
          <Confirm
            label="重启设备"
            title="重启设备？"
            description="屏幕会短暂闪烁，稍后会自动重新连接 Wi-Fi。"
            action={async () => {
              await api("/api/reboot", {});
              setMessage("设备正在重启，请稍后刷新。");
            }}
          />
          <Confirm
            danger
            label="重新配网"
            title="清除所有网络并重新配网？"
            description="设备会断开当前网络并重启。请连接 CuriosityLab-Setup 热点重新设置，称重历史不受影响。"
            action={async () => {
              await api("/api/wifi_forget", {});
              setMessage("设备已进入配网流程。");
            }}
          />
        </div>
      </section>
    </div>
  );
}
function configPatchSafe(a: Config, b: Config) {
  try {
    return configPatch(a, b);
  } catch {
    return { invalid: true };
  }
}
