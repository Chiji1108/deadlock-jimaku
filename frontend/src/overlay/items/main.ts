import { mount } from "svelte";
import { fitSource, ITEMS_SIZE } from "../../lib/overlay-size";
import Items from "./Items.svelte";

// カード（deadlock-ui）の大きさは中で決まっているので、ページごと拡大する
fitSource(ITEMS_SIZE, (scale) => (document.documentElement.style.zoom = String(scale)));

mount(Items, { target: document.getElementById("app")! });
