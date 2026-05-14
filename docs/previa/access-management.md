# Access Management

Previa is open by default. If you do not configure access management, the
effective user is `anonymous` and every route behaves as it did before this
feature.

Protected mode is opt-in and separates browser sessions from automation:

- the app uses JWTs created by `POST /api/v1/auth/login`
- CLI, MCP, scripts, and direct API clients use fixed API tokens

## Start Protected

For CLI-managed contexts:

```bash
printf '%s' 'change-me' | previa up -d --protected --root-username root --root-password-stdin
```

`previa up --protected` persists these values in the context `main.env`:

```env
PREVIA_AUTH_ANONYMOUS=false
PREVIA_ROOT_USERNAME=root
PREVIA_ROOT_PASSWORD=change-me
PREVIA_JWT_SECRET=<generated-if-missing>
```

To explicitly keep a context open:

```bash
previa up -d --anonymous
```

For direct `previa-main` startup, provide the environment yourself:

```bash
PREVIA_AUTH_ANONYMOUS=false \
PREVIA_ROOT_USERNAME=root \
PREVIA_ROOT_PASSWORD=change-me \
PREVIA_JWT_SECRET="$(uuidgen)" \
previa-main
```

Only `/health`, `POST /api/v1/auth/login`, and static app assets are public in
protected mode.

## Login and API Tokens

Browser users log in through the app and receive a JWT.

CLI users should store an API token:

```bash
printf '%s' 'change-me' | previa login --context default --username root --password-stdin
previa whoami --context default
```

You can also create named fixed tokens:

```bash
previa token create --context default --name ci --role operator
previa token list --context default
previa token revoke --context default <token-id>
```

For scripts, set:

```bash
export PREVIA_API_TOKEN='pvk_...'
```

`PREVIA_API_TOKEN` takes precedence over a token saved by `previa login` or
`previa token use`.

## MCP

Protected MCP clients need an API token. You can either use the token stored by
`previa login`:

```bash
previa mcp install codex --context default
```

or reference an environment variable:

```bash
previa mcp install codex --context default --token-env PREVIA_API_TOKEN
previa mcp print cursor --context default --token-env PREVIA_API_TOKEN
```

The generated client config includes an `Authorization` header.

## Roles

- `root`: full access, including creating root/admin users and API tokens.
- `admin`: full access except mutating the environment root account itself.
- `editor`: manages projects, specs, env groups, pipelines, imports/exports,
  executions, and can read runners. Editors cannot manage runners, users, or
  API tokens.
- `operator`: reads data, runs/cancels executions, manages queues, and reads
  runners.
- `viewer`: read-only access.
- `anonymous`: full access only when anonymous mode is enabled.

Runner management is infrastructure-level access. Only `root` and `admin` can
create, update, enable, disable, or delete runner records.

## Access UI

Root and admin users see the access button in the app header. The `/access`
page can:

- create users
- activate, deactivate, and remove users
- create API tokens and display the raw token once
- activate, deactivate, and revoke API tokens

