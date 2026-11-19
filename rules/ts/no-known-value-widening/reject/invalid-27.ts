type Command = () => void; const startCommand = () => {}; const commands: Record<string | 'start', Command> = { start: startCommand };
