export {};
type Payload = { id: string };
type Record<K, V> = Payload;
declare function make(): Payload;
const precise: Payload = make();
const widened: Record<string, unknown> = precise;
const same = widened as Payload;
