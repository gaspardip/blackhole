type Command = () => void; const startCommand = () => {}; type Index<T> = Record<string, T>; type CommandsByName = Index<Command>; const commands: CommandsByName = { start: startCommand };
