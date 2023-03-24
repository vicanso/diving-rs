import en from "./en";
import zh from "./zh";

function i18nGet(name: string) {
  if (window.navigator.language?.includes("zh")) {
    // eslint-disable-next-line @typescript-eslint/ban-ts-comment
    // @ts-ignore
    return zh[name] || "";
  }
  // eslint-disable-next-line @typescript-eslint/ban-ts-comment
  // @ts-ignore
  return en[name] || "";
}
export default i18nGet;
