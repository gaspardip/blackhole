type Item = unknown; type Fallback<Input> = Input extends infer Item ? string : () => Item;
