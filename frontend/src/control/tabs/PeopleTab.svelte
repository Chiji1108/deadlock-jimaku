<script lang="ts">
  // 「プロフィール」のタブ。人の辞書（自分と、よく一緒に遊ぶ人）の、名前・呼ばれ方・アイコンを入れる。
  // 入れた名前は、ヒーローの名前と同じように、字幕で強調してアイコンを付け、翻訳ではその言語の名前にする（DESIGN.md 9.12）。
  // この PC にだけ保存する。変えたらすぐ保存する
  import ImageIcon from "@lucide/svelte/icons/image";
  import LinkIcon from "@lucide/svelte/icons/link";
  import TrashIcon from "@lucide/svelte/icons/trash-2";
  import UserIcon from "@lucide/svelte/icons/user";
  import { toast } from "svelte-sonner";
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import type { Person } from "../../generated/Person";
  import { api } from "../../lib/api";
  import { panel } from "../panel.svelte";

  const MAX_PEOPLE = 50;
  const s = $derived(panel.settings!);
  const people = $derived(s.people);
  const me = $derived(people.find((p) => p.me));
  const others = $derived(people.filter((p) => !p.me));
  // 名前を入れる言語（話す言語・英語・翻訳先）
  const languages = $derived([...new Set([s.micLanguage, "en", ...s.translateTo])]);
  const languageNames = new Intl.DisplayNames(["ja"], { type: "language" });

  /** 足すときの名前（自分、ほかの人） */
  let adding = $state({ me: "", other: "" });
  /** チャンネルの URL（人ごと） */
  let urls = $state<Record<string, string>>({});
  /** アイコンを取っている人 */
  let fetching = $state<string>();

  const save = (next: Person[]) => panel.update({ people: next });
  const change = (id: string, patch: Partial<Person>) =>
    save(people.map((p) => (p.id === id ? { ...p, ...patch } : p)));
  const newId = () => Math.random().toString(36).slice(2, 10);

  function add(isMe: boolean) {
    const name = (isMe ? adding.me : adding.other).trim();
    if (!name) return;
    const person: Person = { id: newId(), me: isMe, names: { [s.micLanguage]: name }, aliases: [] };
    void save(isMe ? [person, ...people] : [...people, person]);
    adding = isMe ? { ...adding, me: "" } : { ...adding, other: "" };
  }

  function rename(person: Person, lang: string, name: string) {
    const names = { ...person.names, [lang]: name.trim() };
    // 名前を全部消すと、その人は保存できない（消すなら「消す」を押す）
    if (!Object.values(names).some(Boolean)) return toast.error("名前を 1 つは入れてください");
    void change(person.id, { names });
  }

  const realias = (person: Person, text: string) =>
    change(person.id, { aliases: text.split(/[、,，\n]/).map((alias) => alias.trim()) });

  async function setIcon(person: Person, get: () => Promise<{ icon: string; name?: string }>) {
    fetching = person.id;
    try {
      const fetched = await get();
      await change(person.id, { icon: fetched.icon });
    } catch (e) {
      toast.error("アイコンを取れませんでした", { description: String(e) });
    } finally {
      fetching = undefined;
    }
  }

  function pick(person: Person, event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (file) void setIcon(person, () => api.savePersonIcon(file));
  }
</script>

{#snippet editor(person: Person)}
  <div class="flex flex-col gap-4 rounded-lg border p-4">
    <div class="flex flex-wrap items-start gap-4">
      <div
        class="flex size-16 shrink-0 items-center justify-center overflow-hidden rounded-full bg-muted"
      >
        {#if person.icon}
          <img class="size-full object-cover" src="/people/{person.icon}" alt="" />
        {:else}
          <UserIcon class="size-7 text-muted-foreground" />
        {/if}
      </div>
      <div class="flex min-w-0 flex-1 flex-col gap-2">
        <div class="flex flex-wrap gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={fetching === person.id}
            onclick={(e) =>
              (e.currentTarget.nextElementSibling as HTMLInputElement | null)?.click()}
          >
            <ImageIcon />画像を選ぶ
          </Button>
          <input
            type="file"
            accept="image/png,image/jpeg,image/webp,image/gif"
            class="hidden"
            onchange={(e) => pick(person, e)}
          />
          {#if person.me && s.rankAccount}
            <Button
              variant="outline"
              size="sm"
              disabled={fetching === person.id}
              onclick={() => setIcon(person, () => api.fetchPersonIcon({ steam: true }))}
            >
              Steam のアイコンを使う
            </Button>
          {/if}
          {#if person.icon}
            <Button
              variant="ghost"
              size="sm"
              onclick={() => change(person.id, { icon: undefined })}
            >
              アイコンを外す
            </Button>
          {/if}
        </div>
        <form
          class="flex gap-2"
          onsubmit={(e) => {
            e.preventDefault();
            const url = urls[person.id]?.trim();
            if (url) void setIcon(person, () => api.fetchPersonIcon({ url }));
          }}
        >
          <Input
            class="flex-1"
            placeholder="Twitch か YouTube のチャンネルの URL"
            bind:value={urls[person.id]}
          />
          <Button
            type="submit"
            variant="outline"
            size="sm"
            disabled={fetching === person.id || !urls[person.id]?.trim()}
          >
            <LinkIcon />アイコンを取る
          </Button>
        </form>
      </div>
      <Button
        variant="ghost"
        size="icon"
        aria-label="この人を消す"
        onclick={() => save(people.filter((p) => p.id !== person.id))}
      >
        <TrashIcon />
      </Button>
    </div>
    <div class="grid gap-3 sm:grid-cols-2">
      {#each languages as lang (lang)}
        <Field.Field>
          <Field.Label for="{person.id}-{lang}"
            >名前（{languageNames.of(lang) ?? lang}）</Field.Label
          >
          <Input
            id="{person.id}-{lang}"
            value={person.names[lang] ?? ""}
            onchange={(e) => rename(person, lang, e.currentTarget.value)}
          />
        </Field.Field>
      {/each}
    </div>
    <Field.Field>
      <Field.Label for="{person.id}-aliases">呼ばれ方</Field.Label>
      <Input
        id="{person.id}-aliases"
        placeholder="あだ名など。「、」で区切る"
        value={person.aliases.join("、")}
        onchange={(e) => realias(person, e.currentTarget.value)}
      />
      <Field.Description>名前と同じように、字幕の中で探します</Field.Description>
    </Field.Field>
  </div>
{/snippet}

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title>自分</Card.Title>
      <Card.Description>
        名前とアイコンを入れると、字幕で自分の名前が、ヒーローの名前と同じように強調されます。翻訳では、その言語の名前になります。デッドロックモードがオンで、話す言語が日本語のときに効きます
      </Card.Description>
    </Card.Header>
    <Card.Content class="flex flex-col gap-4">
      {#if me}
        {@render editor(me)}
      {:else}
        <form
          class="flex gap-2"
          onsubmit={(e) => {
            e.preventDefault();
            add(true);
          }}
        >
          <Input class="flex-1" placeholder="自分の名前" bind:value={adding.me} />
          <Button type="submit" disabled={!adding.me.trim()}>足す</Button>
        </form>
      {/if}
    </Card.Content>
  </Card.Root>

  <Card.Root>
    <Card.Header>
      <Card.Title class="flex items-center gap-2">
        一緒に遊ぶ人
        {#if others.length}<Badge variant="secondary">{others.length}</Badge>{/if}
      </Card.Title>
      <Card.Description>
        よく名前を呼ぶ人を入れておくと、文字起こしが名前を書き間違えにくくなります
      </Card.Description>
    </Card.Header>
    <Card.Content class="flex flex-col gap-4">
      {#each others as person (person.id)}
        {@render editor(person)}
      {/each}
      {#if people.length < MAX_PEOPLE}
        <form
          class="flex gap-2"
          onsubmit={(e) => {
            e.preventDefault();
            add(false);
          }}
        >
          <Input class="flex-1" placeholder="足す人の名前" bind:value={adding.other} />
          <Button type="submit" variant="outline" disabled={!adding.other.trim()}>足す</Button>
        </form>
      {/if}
    </Card.Content>
  </Card.Root>

  <p class="text-sm text-muted-foreground">
    名前とアイコンは、この PC
    にだけ保存します。名前は、選んだモードによっては、用語と同じように、文字起こしや翻訳のサービスに送られます（サーバーを使わない
    Chrome・Chrome+・ローカルのモードでは送りません）。アイコンの画像は送りません
  </p>
</div>
