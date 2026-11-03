import { mount } from "svelte";
import { fitSource, ITEM_CARD_WIDTH, ITEMS_SIZE } from "../../lib/overlay-size";
import Items from "./Items.svelte";

// カード（deadlock-ui）の大きさは中で決まっているので、カードごと拡大する（--scale。Items.svelte）
fitSource(ITEMS_SIZE, (scale) =>
  document.documentElement.style.setProperty(
    "--scale",
    String((scale * ITEMS_SIZE[0]) / ITEM_CARD_WIDTH),
  ),
);

mount(Items, { target: document.getElementById("app")! });
