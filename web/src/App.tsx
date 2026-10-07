import React, { useEffect, useState } from "react";
import {
  Monitor,
  SlidersHorizontal,
  Activity,
  Terminal,
  Cpu,
  House,
  Bluetooth,
} from "lucide-react";
import { usePoll, type System } from "@/lib/api";
import { ScreenPage } from "./pages/screen";
import { SystemPage } from "./pages/system";
import { SettingsPage } from "./pages/settings";
import { LogsPage } from "./pages/logs";
import { HomePage } from "./pages/home";
import { BluetoothPage } from "./pages/bluetooth";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import "./index.css";
const pages = [
  { path: "/bluetooth", name: "蓝牙", title: "蓝牙管理", icon: Bluetooth },
  {
    path: "/",
    name: "屏幕",
    title: "屏幕",
    icon: Monitor,
  },
  {
    path: "/home",
    name: "家居",
    title: "家居设备",
    icon: House,
  },
  {
    path: "/system.html",
    name: "设备",
    title: "设备状态",
    icon: Activity,
  },
  {
    path: "/logs.html",
    name: "日志",
    title: "运行日志",
    icon: Terminal,
  },
  {
    path: "/settings",
    name: "设置",
    title: "设置",
    icon: SlidersHorizontal,
  },

];
export default function App() {
  const [path, setPath] = useState(location.pathname);
  const [dirty, setDirty] = useState(false),
    [pendingPath, setPendingPath] = useState<string | null>(null);
  const system = usePoll<System>("/api/system", 10000);
  const current = pages.find((p) => p.path === path) || pages[0];
  useEffect(() => {
    const handler = () => {
      const next = location.pathname;
      if (dirty) {
        history.pushState(null, "", path);
        if (dirty) setPendingPath(next);
      } else setPath(next);
    };
    addEventListener("popstate", handler);
    return () => removeEventListener("popstate", handler);
  }, [dirty, path]);
  useEffect(() => {
    document.title = `${current.name} · Glance`;
  }, [current.name]);
  function go(to: string) {
    history.pushState(null, "", to);
    setPath(to);
    window.scrollTo(0, 0);
  }
  function navigate(e: React.MouseEvent<HTMLAnchorElement>, to: string) {
    if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
    e.preventDefault();
    if (to === path) return;
    if (dirty) {
      setPendingPath(to);
      return;
    }
    go(to);
  }
  const online = !!system.data && !system.error;
  return (
    <div className="min-h-dvh">
      <aside className="fixed inset-y-0 left-0 z-20 hidden w-56 flex-col border-r bg-white px-5 py-8 lg:flex">
        <a
          href="/"
          onClick={(e) => navigate(e, "/")}
          className="flex items-center gap-3 px-3"
        >
          <span className="flex size-9 items-center justify-center rounded-xl bg-primary text-white">
            <Monitor size={20} />
          </span>
          <span className="text-2xl font-semibold tracking-[-1px]">
            glance<span className="text-primary">.</span>
          </span>
        </a>
        <nav aria-label="主导航" className="mt-8 space-y-1.5">
          {pages.map((p) => (
            <a
              key={p.path}
              href={p.path}
              onClick={(e) => navigate(e, p.path)}
              aria-current={path === p.path ? "page" : undefined}
              className={`flex items-center gap-3 rounded-lg px-3 py-3 text-sm font-medium transition-colors ${path === p.path ? "bg-accent text-primary" : "text-muted-foreground hover:bg-muted hover:text-foreground"}`}
            >
              <p.icon size={18} />
              {p.name}
              {path === p.path && (
                <span className="ml-auto size-1.5 rounded-full bg-primary" />
              )}
            </a>
          ))}
        </nav>
        <div className="mt-auto border-t px-3 pt-5">
          <div className="mb-2 flex items-center gap-2 text-sm font-medium">
            <Cpu size={16} />
            Glance RLCD
          </div>
          <p className="caption">ESP32-S3 · 4.2 英寸</p>
          <p className="mt-3 font-mono text-xs text-muted-foreground">
            {location.host}
          </p>
        </div>
      </aside>
      <div className="lg:ml-56">
        <header className="flex h-14 sm:h-16 items-center justify-between gap-3 border-b bg-white/80 px-4 sm:px-9 lg:px-12">
          <div className="flex items-center gap-2 text-sm">
            <span className="font-semibold lg:hidden">glance.</span>
            <span className="hidden text-muted-foreground lg:inline">
              工作台
            </span>
            <span className="text-border">/</span>
            <span>{current.title}</span>
          </div>
          <div className="flex items-center gap-2 rounded-full border bg-white px-3 py-1.5 text-xs text-muted-foreground">
            <span
              className={`size-1.5 rounded-full ${online ? "bg-primary" : "bg-stone-400"}`}
            />
            {online
                ? "设备已连接"
                : system.error
                  ? "设备离线"
                  : "正在连接"}
            <span className="hidden border-l pl-2 sm:inline">
              {system.data?.wifi_ip?.join(".") || location.hostname}
            </span>
          </div>
        </header>
        <main className="mx-auto max-w-[1360px] px-3 pb-[calc(88px+env(safe-area-inset-bottom))] pt-3 sm:px-9 sm:pt-6 lg:px-12 lg:pb-8">
          <h1 className="sr-only sm:not-sr-only sm:mb-5 sm:text-2xl sm:font-semibold sm:tracking-tight">
            {current.title}
          </h1>
          {path === "/" && <ScreenPage system={system} />}
          {path === "/system.html" && <SystemPage system={system} />}
          {path === "/home" && <HomePage />}
          {path === "/bluetooth" && <BluetoothPage />}
          {path === "/settings" && <SettingsPage onDirty={setDirty} />}
          {path === "/logs.html" && <LogsPage />}
          {!pages.some((p) => p.path === path) && (
            <p>
              没有找到这个页面。<a href="/">返回屏幕</a>
            </p>
          )}
        </main>
      </div>
      <nav
        aria-label="手机导航"
        className="fixed inset-x-0 bottom-0 z-30 grid grid-cols-6 border-t bg-white px-2 pb-[max(8px,env(safe-area-inset-bottom))] pt-2 lg:hidden"
      >
        {pages.map((p) => (
          <a
            key={p.path}
            href={p.path}
            aria-current={path === p.path ? "page" : undefined}
            onClick={(e) => navigate(e, p.path)}
            className={`flex min-w-0 flex-col items-center gap-1 rounded-lg px-1 py-1.5 text-xs ${path === p.path ? "bg-accent text-primary" : "text-muted-foreground"}`}
          >
            <p.icon size={20} />
            {p.name}
          </a>
        ))}
      </nav>
      <Dialog
        open={pendingPath !== null}
        onOpenChange={(v) => {
          if (!v) setPendingPath(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>还有未保存的设置</DialogTitle>
            <DialogDescription>
              离开后，本次修改将不会保存。也可以留下继续编辑。
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setPendingPath(null)}>
              继续编辑
            </Button>
            <Button
              onClick={() => {
                if (pendingPath) {
                  setDirty(false);
                  go(pendingPath);
                  setPendingPath(null);
                }
              }}
            >
              放弃修改并离开
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
export class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { failed: boolean }
> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  render() {
    return this.state.failed ? (
      <div className="m-8 panel p-8">
        <h1 className="text-xl">页面暂时无法显示</h1>
        <p className="my-4">请刷新页面重试。设备会继续正常运行。</p>
        <a href="/" className="underline">
          重新打开
        </a>
      </div>
    ) : (
      this.props.children
    );
  }
}
