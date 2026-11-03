// テストで SQL を文字列として読み込む（vite の ?raw）
declare module "*?raw" {
  const text: string;
  export default text;
}
