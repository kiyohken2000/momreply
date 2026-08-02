import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
} from "react";
import { getUiLanguage, setUiLanguage } from "./api";
import { detectLang, translate, type Key, type Lang } from "./i18n";

/**
 * 表示言語。
 *
 * 保存先は app.db の kv。Rust 側も同じ値を読んで、通知とメニューバーの
 * ツールチップに使う。フロントだけに持たせると、画面は英語なのに通知が
 * 日本語で来ることになる。
 *
 * 読み込みが終わるまでは OS の設定から推定した値で描く。空白を挟むと
 * ポップオーバーが一瞬白くなる。
 */
type Ctx = {
  lang: Lang;
  setLang: (next: Lang) => void;
  t: (key: Key, vars?: Record<string, string | number>) => string;
};

const LangContext = createContext<Ctx | null>(null);

export function LangProvider({ children }: { children: React.ReactNode }) {
  const [lang, setLangState] = useState<Lang>(detectLang);

  useEffect(() => {
    void getUiLanguage()
      .then((saved) => {
        if (saved === "ja" || saved === "en") setLangState(saved);
      })
      .catch(() => {
        // 読めなくても推定値で動く。ここで止める理由が無い。
      });
  }, []);

  const setLang = useCallback((next: Lang) => {
    setLangState(next);
    void setUiLanguage(next).catch(() => {});
  }, []);

  const t = useCallback(
    (key: Key, vars?: Record<string, string | number>) =>
      translate(lang, key, vars),
    [lang],
  );

  return <LangContext value={{ lang, setLang, t }}>{children}</LangContext>;
}

export function useLang(): Ctx {
  const ctx = useContext(LangContext);
  if (!ctx) throw new Error("LangProvider の外で useLang を呼んでいる");
  return ctx;
}
