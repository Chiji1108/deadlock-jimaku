// マイクの名前の突き合わせ。Chrome のラベルは、OS の名前（アプリのマイクの名前）と少し違う

const tidy = (text: string) => text.normalize("NFKC").trim().replace(/\s+/g, " ");

/** 末尾の括弧を 1 つ除く。Chrome は機器の ID や接続の種類（(045e:0810)、(Built-in)）を足す */
const withoutSuffix = (label: string) => label.replace(/\s*\([^()]*\)$/, "");

/** `items` から、Chrome のラベルと OS の名前が同じマイクのものを選ぶ（`pair` が、比べる 2 つを返す）。
 * 部分一致は別のマイクに当たるので、全体を比べる。全体が同じものがなければ、ラベルの末尾の括弧を除いて比べる */
export function namedAlike<T>(items: T[], pair: (item: T) => [label: string, name: string]): T[] {
  const pairs = items.map((item) => {
    const [label, name] = pair(item);
    return { item, label: tidy(label), name: tidy(name) };
  });
  const exact = pairs.filter((p) => p.label === p.name);
  const alike = exact.length ? exact : pairs.filter((p) => withoutSuffix(p.label) === p.name);
  return alike.map((p) => p.item);
}
