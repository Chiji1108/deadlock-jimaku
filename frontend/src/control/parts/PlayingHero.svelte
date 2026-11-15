<script lang="ts">
  // 使っているヒーロー。自分で選ぶか、試合から取ったもの（`panel.view.played`）に任せる（DESIGN.md 9.13）。
  // 試合ごとに変わるので、プロフィールではなくホームに置く
  import * as Card from "$lib/components/ui/card";
  import * as Select from "$lib/components/ui/select";
  import { entityName, glossary, loadGlossary } from "../../lib/glossary.svelte";
  import { panel } from "../panel.svelte";

  const s = $derived(panel.settings!);
  const AUTO = "auto";
  void loadGlossary();
  const heroName = (ref: string | undefined) => {
    const entity = ref ? glossary.entity(ref) : undefined;
    return entity ? entityName(entity, "ja") : undefined;
  };
  const heroes = $derived(
    glossary.heroes
      .map(([ref, entity]) => ({ ref, name: entityName(entity, "ja"), icon: entity.icon }))
      .toSorted((a, b) => a.name.localeCompare(b.name, "ja")),
  );
  /** 自動で取ったヒーロー。選択肢の名前は短くして、中身はここで出す */
  const auto = $derived.by(() => {
    if (!s.rankAccount) return "ランクのタブで Steam のアカウントをつなぐと、試合から取ります";
    return `今は ${heroName(panel.view?.played) ?? "まだ分かりません"}`;
  });
</script>

<Card.Root>
  <Card.Header>
    <Card.Title>使っているヒーロー</Card.Title>
    <Card.Description>
      「ウルト使う」のように名前を言わなくても、このヒーローのスキルとして強調します
    </Card.Description>
  </Card.Header>
  <Card.Content class="flex flex-col gap-2">
    <Select.Root
      type="single"
      value={s.playing ?? AUTO}
      onValueChange={(v) => panel.update({ playing: v === AUTO ? "" : v })}
      onOpenChange={(open) => open && void loadGlossary()}
    >
      <Select.Trigger class="w-full">{heroName(s.playing) ?? "自動"}</Select.Trigger>
      <Select.Content>
        <Select.Item value={AUTO}>自動</Select.Item>
        {#each heroes as hero (hero.ref)}
          <Select.Item value={hero.ref}>
            {#if hero.icon}<img class="size-5 rounded" src={hero.icon} alt="" />{/if}
            {hero.name}
          </Select.Item>
        {/each}
      </Select.Content>
    </Select.Root>
    <p class="text-xs text-muted-foreground">
      {#if !s.playing}自動: {auto}。{/if}自動は、前の試合のヒーローです。用語を AI
      で判定するモードで効きます
    </p>
  </Card.Content>
</Card.Root>
