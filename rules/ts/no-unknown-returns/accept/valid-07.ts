type Key = unknown; type Mapped<Input> = { [Key in keyof Input]: () => Key };
