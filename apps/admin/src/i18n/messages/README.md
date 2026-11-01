# Orveil Console message catalogs

All admin UI copy lives in this directory. Components call `tr('message_key')`; they never contain inline Chinese/English pairs.

## Files

- `zh-CN.ts` — source catalog. Keys are stable identifiers; values are Chinese copy.
- `en-US.ts` — English catalog, typed as `Record<keyof typeof zhCN, string>`, so a missing/extra key fails `tsc`.
- `../translate.ts` — locale registry, interpolation (`{name}`) and fallback to Chinese.

## Add a new message

1. Add the key + Chinese value to `zh-CN.ts`.
2. Add the same key + translation to `en-US.ts`.
3. Use `tr('message_key')` or `tr('message_key', { name })` in the component.

For interpolated messages, use `{name}` placeholders in both catalogs:
`tr('total_events', { total: data.total })`.

## Add a new language

1. Copy `en-US.ts` to e.g. `fr-FR.ts`, translate values, keep keys identical.
2. In `../translate.ts`:
   - add `{ id: 'fr-FR', label: 'Français' }` to `SUPPORTED_LOCALES`;
   - import the catalog and add it to `catalogs`.
3. Done. Components, `LocaleSwitch` and persistence do not need changes; the type system enforces catalog key parity.

## Rules

- Do not add inline `zh/en` pairs in pages/components.
- Do not translate API enum values, IDs, Key prefixes, provider/model names or server-provided error details.
- Keep keys stable: changing a key is a catalog migration, not a component edit.
