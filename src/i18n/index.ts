import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./en.json";
import zh from "./zh.json";
import walletEn from "./wallet-en.json";
import walletZh from "./wallet-zh.json";
import authEn from "./auth-en.json";
import authZh from "./auth-zh.json";

export function initI18n() {
  if (i18n.isInitialized) return;
  void i18n
    .use(initReactI18next)
    .init({
      resources: {
        en: { share: en, wallet: walletEn, auth: authEn },
        zh: { share: zh, wallet: walletZh, auth: authZh },
      },
      lng: "zh",
      fallbackLng: "en",
      defaultNS: "share",
      interpolation: { escapeValue: false },
    });
}

export default i18n;
