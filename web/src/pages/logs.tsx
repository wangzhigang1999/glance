import { useEffect, useRef, useState } from "react";
import { Pause, Play, Search, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/api";
import { logLevel } from "@/lib/policy.mjs";
import { Notice } from "@/components/common";
export function LogsPage() {
  const [lines, setLines] = useState<string[]>([]),
    [error, setError] = useState(""),
    [paused, setPaused] = useState(false),
    [follow, setFollow] = useState(true),
    [level, setLevel] = useState("ALL"),
    [query, setQuery] = useState("");
  const since = useRef(0),
    box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (paused) return;
    let alive = true,
      timer: ReturnType<typeof setTimeout>;
    const controller = new AbortController();
    async function poll() {
      if (!document.hidden) {
        try {
          const j = await api<{ next: number; lines: string[] }>(
            `/logs.json?since=${since.current}`,
            undefined,
            AbortSignal.any([controller.signal, AbortSignal.timeout(8000)]),
          );
          if (alive) {
            if (j.next < since.current) {
              since.current = 0;
              setLines(["I logs: 设备已重启，重新读取日志"]);
            } else {
              const gap =
                since.current > 0
                  ? Math.max(0, j.next - since.current - j.lines.length)
                  : 0;
              since.current = j.next;
              setLines((old) =>
                [
                  ...old,
                  ...(gap ? [`W logs: 设备日志缓存已覆盖 ${gap} 条记录`] : []),
                  ...j.lines,
                ].slice(-1200),
              );
            }
            setError("");
          }
        } catch (e) {
          if (alive) setError("暂时无法获取日志，正在重新连接…");
        }
      }
      if (alive) timer = setTimeout(poll, 1200);
    }
    void poll();
    return () => {
      alive = false;
      clearTimeout(timer);
      controller.abort();
    };
  }, [paused]);
  const shown = lines.filter(
    (l) =>
      (level === "ALL" || logLevel(l) === level) &&
      l.toLowerCase().includes(query.toLowerCase()),
  );
  useEffect(() => {
    if (follow && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [lines, follow, level, query]);
  return (
    <div className="space-y-4">
      <Notice error>{error}</Notice>
      <section className="panel overflow-hidden">
        <div className="flex flex-wrap items-center gap-2 border-b p-3 sm:p-5">
          <div className="relative min-w-0 w-full sm:w-auto sm:flex-1">
            <Search
              size={16}
              className="absolute left-3 top-3 text-muted-foreground"
            />
            <Input
              aria-label="搜索日志"
              className="pl-9"
              placeholder="搜索关键词…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>
          <select
            aria-label="日志级别"
            className="native-select flex-1 sm:flex-none"
            value={level}
            onChange={(e) => setLevel(e.target.value)}
          >
            {[
              ["ALL", "全部级别"],
              ["E", "错误"],
              ["W", "警告"],
              ["I", "信息"],
              ["D", "调试"],
              ["V", "详细"],
            ].map(([v, t]) => (
              <option key={v} value={v}>
                {t}
              </option>
            ))}
          </select>
          <Button variant="outline" onClick={() => setPaused((v) => !v)}>
            {paused ? <Play /> : <Pause />}
            {paused ? "继续" : "暂停"}
          </Button>
          <Button
            variant="ghost"
            size="icon"
            aria-label="清空显示的日志"
            onClick={() => setLines([])}
          >
            <Trash2 />
          </Button>
        </div>
        <div className="flex flex-wrap items-center justify-between gap-3 border-b bg-muted/40 px-3 py-3 sm:px-5 text-xs text-muted-foreground">
          <span className="flex items-center gap-2">
            <span
              className={`size-1.5 rounded-full ${paused ? "bg-amber-500" : error ? "bg-red-400" : "bg-primary"}`}
            />
            {paused ? "已暂停读取" : "实时日志"} · 显示 {shown.length} /{" "}
            {lines.length} 条
          </span>
          <label className="flex items-center gap-2">
            自动滚动
            <Switch size="sm" checked={follow} onCheckedChange={setFollow} />
          </label>
        </div>
        <div
          ref={box}
          tabIndex={0}
          aria-label="日志内容"
          className="h-[max(240px,calc(100dvh-320px))] sm:h-[min(560px,60dvh)] overflow-auto bg-[#202720] p-4 font-mono text-[13px] leading-6 text-[#cad3c5] sm:p-5"
        >
          {shown.length ? (
            shown.map((line, i) => (
              <div
                key={i}
                className={`whitespace-pre-wrap break-all border-b border-white/[.04] ${logLevel(line) === "E" ? "text-[#ffa99f]" : logLevel(line) === "W" ? "text-[#ebd297]" : ""}`}
              >
                {line}
              </div>
            ))
          ) : (
            <p className="py-10 text-center text-[#8d9b87]">
              {lines.length ? "没有匹配的日志" : "等待设备日志…"}
            </p>
          )}
        </div>
        <p className="caption px-3 py-3 sm:px-5 sm:py-4">
          保留最近 1200 条。清空只影响本页显示，不会删除设备上的记录。
        </p>
      </section>
    </div>
  );
}
