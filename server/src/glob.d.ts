// テストで migrations/ の SQL を文字列として読み込む（vite の import.meta.glob と ?raw）
interface ImportMeta {
  glob<T>(pattern: string, options: { query: "?raw"; import: "default"; eager: true }): Record<string, T>;
}
