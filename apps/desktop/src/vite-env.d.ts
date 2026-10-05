/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** 置为 '1' 时用假数据在浏览器里预览界面 */
  readonly VITE_MOCK?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
