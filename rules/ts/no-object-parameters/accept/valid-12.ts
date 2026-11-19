type Item = object; type Unpacked<Input> = Input extends Promise<infer Item> ? (value: Item) => void : never;
