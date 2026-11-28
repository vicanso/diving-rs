import en from "./en";
import zh from "./zh";

export const getLocale = (): "en" | "zh" =>
  typeof window !== "undefined" &&
  window.navigator.language?.toLowerCase().includes("zh")
    ? "zh"
    : "en";

function i18nGet(name: string) {
  if (getLocale() === "zh") {
    // eslint-disable-next-line @typescript-eslint/ban-ts-comment
    // @ts-ignore
    return zh[name] ?? en[name] ?? "";
  }
  // eslint-disable-next-line @typescript-eslint/ban-ts-comment
  // @ts-ignore
  return en[name] || "";
}
export default i18nGet;
