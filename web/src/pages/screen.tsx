import { useEffect, useRef, useState } from "react";
import {
  Maximize2,
  Pause,
  Play,
  RefreshCw,
  SkipForward,
  Thermometer,
  Droplets,
  Wifi,
  Radio,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Notice } from "@/components/common";
import { api, reading, uptime, type System, type usePoll } from "@/lib/api";
type Props = { system: ReturnType<typeof usePoll<System>> };
export function ScreenPage({ system }: { system: Props["system"] }) {
  const [paused, setPaused] = useState(false),
    [rate, setRate] = useState(1000),
    [nonce, setNonce] = useState(0);
  const [src, setSrc] = useState(""),
    [error, setError] = useState(""),
    [latency, setLatency] = useState(0),
    [frames, setFrames] = useState(0),
    [nextBusy, setNextBusy] = useState(false);
  const frame = useRef<HTMLDivElement>(null),
    objectUrl = useRef("");
  useEffect(() => {
    if (paused) return;
    let alive = true,
      timer: ReturnType<typeof setTimeout>;
    const controller = new AbortController();
    async function tick() {
      if (!document.hidden) {
        try {
          const start = performance.now();
          const response = await fetch(`/screen.bmp?t=${Date.now()}`, {
            cache: "no-store",
            signal: AbortSignal.any([
              controller.signal,
              AbortSignal.timeout(7000),
            ]),
          });
          if (!response.ok) throw Error("屏幕暂时无法读取");
          const blob = await response.blob();
          if (alive) {
            const url = URL.createObjectURL(blob);
            if (objectUrl.current) URL.revokeObjectURL(objectUrl.current);
            objectUrl.current = url;
            setSrc(url);
            setLatency(Math.round(performance.now() - start));
            setFrames((n) => n + 1);
            setError("");
          }
        } catch (e) {
          if (alive) setError("屏幕连接中断，正在重试…");
        }
      }
      if (alive) timer = setTimeout(tick, rate);
    }
    void tick();
    return () => {
      alive = false;
      clearTimeout(timer);
      controller.abort();
    };
  }, [paused, rate, nonce]);
  useEffect(
    () => () => {
      if (objectUrl.current) URL.revokeObjectURL(objectUrl.current);
    },
    [],
  );
  async function next() {
    if (nextBusy) return;
    setNextBusy(true);
    try {
      await api("/next", {});
      setNonce((n) => n + 1);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setNextBusy(false);
    }
  }
  const s = system.data;
  return (
    <div className="space-y-4 sm:space-y-6">
      <div className="grid grid-cols-1 gap-4 sm:gap-6 xl:grid-cols-[minmax(0,1fr)_280px]">
        <section className="panel overflow-hidden">
          <div className="flex items-center justify-between border-b px-4 py-3 sm:px-5 sm:py-4">
            <h2 className="section-title">实时屏幕</h2>
            <span className="flex items-center gap-2 text-xs text-muted-foreground">
              <span
                className={`size-1.5 rounded-full ${error ? "bg-red-400" : paused ? "bg-amber-400" : "bg-primary"}`}
              />
              {error
                ? "连接中断"
                : paused
                  ? "已暂停"
                  : src
                    ? "实时同步"
                    : "正在连接"}
            </span>
          </div>
          <div
            ref={frame}
            className="soft-grid flex items-center justify-center bg-[#f1f3ed] p-2 sm:min-h-[420px] sm:p-10"
          >
            <div className="w-full max-w-[540px] rounded-xl sm:rounded-[20px] border border-[#bbc2b4] bg-[#e0e4d9] p-1.5 shadow-[0_8px_24px_-12px_#67715a] sm:p-5">
              <div className="relative aspect-[4/3] overflow-hidden rounded-md border border-[#aeb7a3] bg-[#e4e8d8]">
                {src ? (
                  <img
                    src={src}
                    width="400"
                    height="300"
                    alt="板子当前屏幕"
                    className="h-full w-full object-contain mix-blend-multiply"
                    style={{ imageRendering: "pixelated" }}
                  />
                ) : (
                  <div
                    role="status"
                    className="grid h-full place-items-center text-sm text-muted-foreground"
                  >
                    正在读取屏幕…
                  </div>
                )}
                {error && (
                  <div className="absolute inset-0 grid place-items-center bg-white/80 px-6 text-center text-sm">
                    {error}
                  </div>
                )}
              </div>
            </div>
          </div>
          <div className="flex flex-wrap items-center justify-between gap-2 border-t p-3 sm:p-5">
            <div className="grid w-full grid-cols-2 gap-2 sm:flex sm:w-auto">
              <Button onClick={next} disabled={nextBusy}>
                <SkipForward />
                {nextBusy ? "切换中" : "下一页"}
              </Button>
              <Button variant="outline" onClick={() => setPaused((p) => !p)}>
                {paused ? <Play /> : <Pause />}
                {paused ? "继续" : "暂停"}
              </Button>
            </div>
            <div className="flex w-full items-center gap-2 sm:w-auto">
              <label htmlFor="screen-rate" className="sr-only">
                屏幕刷新间隔
              </label>
              <select
                id="screen-rate"
                className="native-select min-w-0 flex-1 sm:flex-none"
                value={rate}
                onChange={(e) => setRate(+e.target.value)}
              >
                {[500, 1000, 2000, 5000].map((n) => (
                  <option key={n} value={n}>
                    {n / 1000} 秒刷新
                  </option>
                ))}
              </select>
              <Button
                size="icon"
                variant="ghost"
                aria-label="刷新屏幕"
                onClick={() => {
                  setPaused(false);
                  setNonce((n) => n + 1);
                }}
              >
                <RefreshCw />
              </Button>
              <Button
                size="icon"
                variant="ghost"
                aria-label="全屏查看"
                onClick={() => {
                  const p = document.fullscreenElement
                    ? document.exitFullscreen()
                    : frame.current?.requestFullscreen();
                  p?.catch(() => setError("浏览器不支持全屏，请使用横屏查看"));
                }}
              >
                <Maximize2 />
              </Button>
            </div>
          </div>
          <div className="flex flex-wrap justify-between gap-x-2 gap-y-1 border-t px-3 py-2 sm:px-5 sm:py-3 font-mono text-[11px] text-muted-foreground">
            <span>400 × 300 · 1-bit</span>
            <span>
              {latency || "—"} ms · {frames} 帧
            </span>
          </div>
        </section>
        <aside className="flex flex-col gap-3 sm:gap-5">
          <section className="panel p-4 sm:p-5">
            <div className="mb-3 sm:mb-5 flex items-center gap-2 text-sm font-semibold">
              <Radio size={16} className="text-primary" />
              环境
            </div>
            <div className="grid grid-cols-2 gap-4 xl:grid-cols-1">
              <div className="xl:border-b xl:pb-5">
                <p className="caption flex items-center gap-2">
                  <Thermometer size={15} />
                  温度
                </p>
                <p className="mt-2 text-3xl sm:text-4xl font-medium tracking-tight tabular-nums">
                  {reading(s?.temp_c)}
                  <span className="ml-1 text-lg text-muted-foreground">°C</span>
                </p>
              </div>
              <div>
                <p className="caption flex items-center gap-2">
                  <Droplets size={15} />
                  湿度
                </p>
                <p className="mt-2 text-3xl sm:text-4xl font-medium tracking-tight tabular-nums">
                  {reading(s?.humid_pct, 0)}
                  <span className="ml-1 text-lg text-muted-foreground">%</span>
                </p>
              </div>
            </div>
            <p className="caption mt-3 border-t pt-3 sm:mt-5 sm:pt-4">
              {system.error
                ? "连接中断 · 显示最后一次读数"
                : s?.sampled_at
                  ? `采样于 ${new Date(s.sampled_at * 1000).toLocaleTimeString("zh-CN")}`
                  : "等待传感器采样"}
            </p>
          </section>
          <section className="rounded-xl border border-[#dce5d4] bg-[#f0f4eb] p-4 sm:p-5">
            <div className="flex items-center gap-2 text-sm font-semibold text-primary">
              <Wifi size={16} />
              网络
            </div>
            <p className="mt-3 break-all text-sm font-medium">
              {s?.wifi_ssid || "读取网络中…"}
            </p>
            <p className="caption mt-2">已运行 {uptime(s?.uptime_s)}</p>
          </section>
        </aside>
      </div>
      <Notice error>
        {system.error ? "设备状态暂时无法刷新，请检查 Wi-Fi 连接。" : ""}
      </Notice>
    </div>
  );
}
