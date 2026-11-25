<script lang="ts">
  // Steam のアカウント。ランクを取るのに使うので、「ランク」のタブに置く（プロフィールの自分のアイコンも、ここから取れる）
  import Help from "./Help.svelte";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import { panel } from "../panel.svelte";

  const saved = () => panel.settings?.rankAccount?.toString() ?? "";
  let account = $state(saved());

  /** サーバーは SteamID64 も 32bit の ID に直して保存するので、保存した値を入力欄に戻す */
  async function save() {
    // 失敗したら直せるよう、入力は残す
    if (await panel.update({ rankAccount: account })) account = saved();
  }
</script>

<Card.Root>
  <Card.Header>
    <Card.Title class="flex items-center gap-1.5"
      >Steam のアカウント<Help>つなぐと、ランクのオーバーレイが出ます</Help></Card.Title
    >
  </Card.Header>
  <Card.Content>
    <form
      onsubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <Field.Field>
        <Field.Label for="account"
          >フレンドコード<Help
            >Steam の「フレンド」→「フレンドを追加」に出ている数字です（SteamID64
            でも可）。空にすると、つながりを切ります</Help
          ></Field.Label
        >
        <div class="flex gap-2">
          <Input
            id="account"
            bind:value={account}
            placeholder="フレンドコード"
            inputmode="numeric"
            autocomplete="off"
          />
          <Button type="submit" disabled={account === saved()}>保存</Button>
        </div>
      </Field.Field>
    </form>
  </Card.Content>
</Card.Root>
