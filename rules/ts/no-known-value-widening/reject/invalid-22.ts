type Command = () => void; const startCommand = () => {}; type Identity<T> = T; const commands: Identity<Record<string, Command>> = { start: startCommand };
