type Command = () => void; const startCommand = () => {}; const commands = { start: startCommand } as Record<string, Command>;
