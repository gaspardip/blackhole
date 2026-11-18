type Command = () => void; const startCommand = () => {}; type Key = string; const commands: Record<Key, Command> = { start: startCommand };
