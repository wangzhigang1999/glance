import { Plug, RefreshCw } from "lucide-react";
import { DeviceOnboarding } from "@/components/device-onboarding";
import { usePoll } from "@/lib/api";
import { Notice, Loading } from "@/components/common";
import { Button } from "@/components/ui/button";
type Device = {
  id: string;
  name: string;
  model: string;
  ip: string;
  online: boolean;
  generation: number;
  sampled_at: number | null;
  values: Record<string, number | boolean | null>;
  error: string;
  firmware: string | null;
};
const labels: Record<string, [string, string]> = {
  motor_speed_rpm: ["电机转速", "rpm"],
  water_pct: ["剩余水量", "%"],
  air_quality_level: ["空气质量等级", ""],
  on: ["设备状态", ""],
  power_w: ["实时功率", "W"],
  temperature_c: ["温度", "°C"],
  humidity_pct: ["湿度", "%"],
  pm25_ug_m3: ["PM2.5", "μg/m³"],
  filter_pct: ["滤芯剩余", "%"],
  brightness_pct: ["亮度", "%"],
  color_temperature_k: ["色温", "K"],
  target_temperature_c: ["设定温度", "°C"],
  energy_10wh: ["设备累计电量", "kWh"],
};
function display(key: string, value: number | boolean | null) {
  if (value == null) return "—";
  if (typeof value === "boolean") return value ? "开启" : "关闭";
  return `${key === "energy_10wh" ? (value / 100).toFixed(2) : Number(value.toFixed(1))} ${labels[key]?.[1] || ""}`;
}
export function HomePage() {
  const state = usePoll<{
    devices: Device[];
    error: string;
    poll_interval_s: number;
  }>("/api/home-devices", 10000);
  return (
    <div className="space-y-5">
      <div className="panel flex items-center justify-between gap-4 p-5">
        <div>
          <p className="font-medium">家里的设备</p>
          <p className="caption mt-1">
            由 Glance 每分钟采集，无需电脑保持开机。
          </p>
        </div>
        <Button
          variant="outline"
          size="icon"
          aria-label="刷新显示"
          onClick={state.refresh}
        >
          <RefreshCw size={16} />
        </Button>
      </div>
      <DeviceOnboarding refresh={state.refresh} />
      <Notice error>
        {state.error
          ? "暂时无法连接 Glance，以下数据可能已经过期。"
          : state.data?.error}
      </Notice>
      {!state.data ? (
        <Loading />
      ) : state.data.devices.length === 0 ? (
        <div className="panel p-6">还没有配置米家设备。</div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {state.data.devices.map((d) => {
            const stale =
              !!state.error ||
              !d.online ||
              (d.sampled_at != null && Date.now() / 1000 - d.sampled_at > 150);
            return (
              <section className="panel p-5" key={d.id}>
                <div className="flex items-start justify-between gap-2">
                  <h2 className="flex items-center gap-2 font-semibold">
                    <Plug size={17} />
                    {d.name}
                  </h2>
                  <span
                    className={`rounded-full px-2 py-1 text-xs ${stale ? "bg-muted text-muted-foreground" : "bg-accent text-primary"}`}
                  >
                    {!d.generation ? "等待采集" : stale ? "暂未更新" : "已连接"}
                  </span>
                </div>
                {stale ? (
                  <p className="mt-5 text-sm text-muted-foreground">
                    {!d.generation
                      ? "正在等待首次读取。"
                      : "暂时没有新数据，稍后会自动重试。"}
                  </p>
                ) : (
                  <dl className="mt-4 space-y-3">
                    {Object.entries(d.values).map(([key, value]) => (
                      <div
                        key={key}
                        className="flex justify-between gap-4 text-sm"
                      >
                        <dt className="text-muted-foreground">
                          {labels[key]?.[0] || key}
                        </dt>
                        <dd
                          className={
                            key === "power_w"
                              ? "font-semibold text-primary"
                              : "font-medium"
                          }
                        >
                          {display(key, value)}
                        </dd>
                      </div>
                    ))}
                    {d.firmware && (
                      <div className="flex justify-between text-sm">
                        <dt className="text-muted-foreground">网关固件</dt>
                        <dd>{d.firmware}</dd>
                      </div>
                    )}
                  </dl>
                )}
                <p className="mt-5 border-t pt-3 text-xs text-muted-foreground">
                  {d.sampled_at
                    ? `最近尝试 ${new Date(d.sampled_at * 1000).toLocaleTimeString("zh-CN")}`
                    : "尚未采集"}{" "}
                  · {d.ip}
                </p>
              </section>
            );
          })}
        </div>
      )}
      <p className="caption">
        摄像头仅显示开关状态。此页不会启动视频或操作设备开关。
      </p>
    </div>
  );
}
