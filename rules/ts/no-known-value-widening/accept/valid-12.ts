type Command = () => void; const startCommand = () => {}; const commands = { start: startCommand } satisfies Record<string, Command>;
