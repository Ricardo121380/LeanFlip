import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./en";
import zh from "./zh";

export function systemLanguage(): "zh" | "en" {
  const nav = typeof navigator !== "undefined" ? navigator.language : "zh";
  return nav.toLowerCase().startsWith("zh") ? "zh" : "en";
}

void i18n.use(initReactI18next).init({
  resources: { zh: { translation: zh }, en: { translation: en } },
  lng: systemLanguage(),
  fallbackLng: "zh",
  interpolation: { escapeValue: false },
});

export function applyLanguage(language: "zh" | "en" | null | undefined) {
  const next = language ?? systemLanguage();
  if (i18n.language !== next) void i18n.changeLanguage(next);
}

export default i18n;
