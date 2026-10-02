import { useEffect, useRef, useState } from "react";
import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Notice } from "@/components/common";

const CLOUD = "https://health.bupt.site";

export function DeviceOnboarding({ refresh }: { refresh: () => void }) {
  const [open, setOpen] = useState(false);
  const [key, setKey] = useState("");
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<unknown[] | null>(null);
  const pairing = useRef<{ window: Window; nonce: string } | null>(null);

  useEffect(() => {
    const receive = (event: MessageEvent) => {
      const active = pairing.current;
      if (!active || event.origin !== CLOUD || event.source !== active.window)
        return;
      const payload = event.data;
      if (
        payload?.type !== "glance-devices" ||
        payload.nonce !== active.nonce ||
        !Array.isArray(payload.devices) ||
        !payload.devices.length ||
        payload.devices.length > 16
      )
        return;
      pairing.current = null;
      setPending(payload.devices);
      setMessage(`已选择 ${payload.devices.length} 台设备，保存后开始采集。`);
    };
    window.addEventListener("message", receive);
    return () => window.removeEventListener("message", receive);
  }, []);

  function authorize() {
    const nonce = Array.from(crypto.getRandomValues(new Uint8Array(24)), (b) =>
      b.toString(16).padStart(2, "0"),
    ).join("");
    const hash = new URLSearchParams({ origin: location.origin, nonce });
    const popup = window.open(
      `${CLOUD}/devices/add#${hash}`,
      "glance-device-setup",
      "width=620,height=800",
    );
    if (!popup) {
      setMessage("请允许弹出窗口后重试。");
      return;
    }
    pairing.current = { window: popup, nonce };
    setMessage("在新窗口中用米家 App 扫码，然后选择设备。");
  }

  async function save() {
    setBusy(true);
    try {
      const response = await fetch("/api/home-devices/config", {
        method: "POST",
        headers: { "Content-Type": "application/json", "X-Admin-Key": key },
        body: JSON.stringify(pending),
      });
      const body = await response.json();
      if (!response.ok) throw new Error(body.error || "保存失败，请重试");
      setPending(null);
      setKey("");
      setMessage("已保存，稍后自动开始采集，无需重启。");
      refresh();
    } catch (error) {
      setMessage(
        error instanceof Error ? error.message : "暂时无法连接板子，请重试。",
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="panel p-5 space-y-4">
      <Button variant="outline" onClick={() => setOpen(!open)}>
        <Plus size={16} />
        添加设备
      </Button>
      {open && (
        <>
          <p className="caption">
            米家扫码授权后选择设备。支持的型号无需重新刷固件；IP
            变化后会自动重新发现。
          </p>
          <Button onClick={authorize} disabled={busy}>
            扫码选择米家设备
          </Button>
          {pending && (
            <div className="space-y-3 max-w-md">
              <label htmlFor="device-admin-key" className="text-sm">
                板子管理口令
              </label>
              <Input
                id="device-admin-key"
                type="password"
                autoComplete="off"
                value={key}
                onChange={(event) => setKey(event.target.value)}
                placeholder="设备管理口令"
              />
              <Button
                onClick={save}
                disabled={busy || !/^[0-9a-fA-F]{32}$/.test(key)}
              >
                {busy ? "正在保存…" : "保存到板子"}
              </Button>
              <Button
                variant="ghost"
                disabled={busy}
                onClick={() => {
                  setPending(null);
                  setKey("");
                  setMessage("");
                }}
              >
                取消
              </Button>
            </div>
          )}
          <Notice>{message}</Notice>
        </>
      )}
    </section>
  );
}
