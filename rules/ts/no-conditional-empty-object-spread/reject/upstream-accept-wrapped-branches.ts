const row = { ...((source?.[`enabled`] ? ({} as Fields) : source['fields']) as Fields) };
