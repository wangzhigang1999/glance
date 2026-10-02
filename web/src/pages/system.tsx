import { RefreshCw, Wifi, Cpu, Clock3, Thermometer } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { Section, Rows, Loading, Notice } from "@/components/common";
import { bytes, uptime, reading, usePoll, type System } from "@/lib/api";
export function SystemPage({
  system,
}: {
  system: ReturnType<typeof usePoll<System>>;
}) {
  const s = system.data;
  const telemetry = usePoll<{
    configured: boolean;
    connected: boolean;
    queued_weight: number;
    climate_dropped: number;
    error: string;
  }>("/api/telemetry", 10000);
  if (!s)
    return (
      <>
        <Notice error>{system.error}</Notice>
        <Loading />
      </>
    );
  const resources: [string, number, number][] = [
    ["SRAM", s.heap_total - s.heap_free, s.heap_total],
    ["PSRAM", s.psram_total - s.psram_free, s.psram_total],
    ["固件分区", s.app_used, s.app_size],
  ];
  return (
    <div className="space-y-4 sm:space-y-6">
      <Notice error>
        {system.error ? "连接中断，以下是最后一次收到的设备状态。" : ""}
      </Notice>
      <div className="grid grid-cols-2 gap-3 sm:gap-5 xl:grid-cols-4">
        {[
          { label: "运行时间", value: uptime(s.uptime_s), icon: Clock3 },
          {
            label: "Wi-Fi 信号",
            value: s.wifi_rssi == null ? "—" : `${s.wifi_rssi} dBm`,
            icon: Wifi,
          },
          {
            label: "芯片温度",
            value: `${reading(s.chip_temp_c)} °C`,
            icon: Thermometer,
          },
          { label: "固件版本", value: `v${s.fw}`, icon: Cpu },
        ].map((m) => (
          <div key={m.label} className="panel min-w-0 p-3 sm:p-5">
            <p className="caption flex items-center justify-between gap-2">
              {m.label}
              <m.icon size={16} className="shrink-0" />
            </p>
            <p className="mt-2 text-base font-semibold tracking-tight sm:mt-4 sm:text-xl">
              {m.value}
            </p>
          </div>
        ))}
      </div>
      <div className="grid items-start gap-4 sm:gap-6 xl:grid-cols-2">
        <Section
          title="连接与同步"
          action={
            <Button
              size="icon"
              variant="ghost"
              aria-label="刷新设备状态"
              onClick={() => {
                system.refresh();
                telemetry.refresh();
              }}
            >
              <RefreshCw />
            </Button>
          }
        >
          <Rows
            items={[
              ["Wi-Fi", s.wifi_connected ? "已连接" : "未连接"],
              ["网络名称", s.wifi_ssid],
              ["IP 地址", s.wifi_ip?.join(".")],
              [
                "数据上报",
                telemetry.error
                  ? "暂时无法读取"
                  : !telemetry.data
                    ? "读取中…"
                    : !telemetry.data.configured
                      ? "未配置"
                      : telemetry.data.connected
                        ? "已连接"
                        : "连接中",
              ],
              ["待同步称重", telemetry.data?.queued_weight ?? "—"],
              [
                "时钟来源",
                s.clock_source === "sntp"
                  ? "网络校时"
                  : s.clock_source === "rtc"
                    ? "本地时钟"
                    : "未同步",
              ],
            ]}
          />
          {telemetry.data?.error && (
            <p className="caption mt-3 break-words">{telemetry.data.error}</p>
          )}
        </Section>
        <Section title="内存与存储">
          <div className="space-y-4 sm:space-y-6">
            {resources.map(([name, used, total]) => (
              <div key={name}>
                <div className="mb-3 flex flex-wrap justify-between gap-x-3 gap-y-1 text-sm">
                  <span>{name}</span>
                  <span className="text-muted-foreground">
                    {bytes(used)} / {bytes(total)}
                  </span>
                </div>
                <Progress
                  aria-label={`${name}使用率`}
                  value={total ? Math.min(100, (used / total) * 100) : 0}
                />
              </div>
            ))}
          </div>
          <div className="mt-5">
            <Rows
              items={[
                ["最低空闲 SRAM", bytes(s.heap_min)],
                ["最大连续空闲块", bytes(s.heap_largest)],
                ["主线程栈余量", bytes(s.stack_hwm)],
                ["Flash 总容量", bytes(s.flash_total)],
              ]}
            />
          </div>
        </Section>
        <Section title="环境与供电">
          <Rows
            items={[
              ["温度", `${reading(s.temp_c, 2)} °C`],
              ["湿度", `${reading(s.humid_pct, 2)} %`],
              ["温度校正", `${s.temp_off_c} °C`],
              ["湿度校正", `${s.humid_off_pct} %`],
              [
                "USB 主机",
                s.usb_plugged == null
                  ? "未知"
                  : s.usb_plugged
                    ? "已检测到"
                    : "未检测到",
              ],
              [
                "电池端电压",
                s.battery_mv == null
                  ? "—"
                  : `${(s.battery_mv / 1000).toFixed(2)} V`,
              ],
            ]}
          />
          <p className="caption mt-4">端电压不代表剩余电量或充电状态。</p>
        </Section>
        <Section title="设备信息">
          <Rows
            items={[
              ["设备型号", "ESP32-S3-RLCD-4.2"],
              ["构建版本", s.revision],
              ["ESP-IDF", s.idf],
              ["MAC 标识", s.mac],
              ["重启原因", s.reset_reason],
              ["采样次数", s.sample_count],
              ["应用位置", `0x${s.app_part_addr.toString(16)}`],
            ]}
          />
        </Section>
      </div>
    </div>
  );
}
