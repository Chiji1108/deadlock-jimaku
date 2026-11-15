<script lang="ts">
  // Steam のアカウント。ランク、使っているヒーロー、自分のアイコンが、ここから取る。
  // 1 つのオーバーレイのものではないので、「プロフィール」に置く
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

<Card.Root id="steam" class="scroll-mt-6">
  <Card.Header>
    <Card.Title>Steam のアカウント</Card.Title>
    <Card.Description>
      つなぐと、ランクのオーバーレイが出て、使っているヒーローを試合から取ります
    </Card.Description>
  </Card.Header>
  <Card.Content>
    <form
      onsubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <Field.Field>
        <Field.Label for="account">フレンドコード</Field.Label>
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
        <Field.Description>
          Steam の「フレンド」→「フレンドを追加」に出ている数字です（SteamID64
          でも可）。空にすると、つながりを切ります
        </Field.Description>
      </Field.Field>
    </form>
  </Card.Content>
</Card.Root>
