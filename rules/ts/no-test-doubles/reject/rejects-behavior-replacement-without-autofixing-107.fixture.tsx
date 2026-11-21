import { vi } from "vitest"; const method = "fn"; const { [method]: create } = vi; create(() => 1);
