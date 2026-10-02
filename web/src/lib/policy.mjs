const numeric = [
  "gh_refresh_s",
  "gh_err_s",
  "sensor_refresh_s",
  "auto_rotate_s",
  "temp_off_c",
  "humid_off_pct",
  "tz_off_s",
  "splash_flash",
];
const editable = ["gh_user", "gh_token", "auto_rotate", ...numeric];
export function configPatch(original, edited) {
  const patch = {};
  for (const key of editable) {
    if (edited[key] === undefined || edited[key] === original[key]) continue;
    if (key === "gh_token" && String(edited[key]).startsWith("***")) continue;
    if (numeric.includes(key)) {
      if (
        String(edited[key]).trim() === "" ||
        !Number.isFinite(Number(edited[key]))
      )
        throw Error("数值设置不能为空");
      patch[key] = Number(edited[key]);
    } else patch[key] = edited[key];
  }
  return patch;
}
export function logLevel(line) {
  return line.match(/^\s*(?:\d+\s+)?([EWIDV])(?:\s|\()/)?.[1] || "I";
}
