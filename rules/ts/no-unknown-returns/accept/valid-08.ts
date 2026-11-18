type Item = unknown; type Unpacked<Input> = Input extends Promise<infer Item> ? () => Item : never;
