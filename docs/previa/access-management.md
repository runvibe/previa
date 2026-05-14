# Access Management

Previa keeps the historical local-development behavior by default: if access
management is not configured, the effective user is `anonymous` and has full
access to the app, API, CLI workflows, and MCP tools.

Access management becomes active only when anonymous access is disabled. In
that protected mode, Previa separates interactive browser sessions from
automation:

- the app uses short-lived JWTs created by `POST /api/v1/auth/login`
- CLI, MCP, CI, scripts, and direct API clients use fixed API tokens

## Modes

### Anonymous Full Access

Anonymous mode is the default. It is equivalent to the previous unprotected
runtime behavior.

```env
PREVIA_AUTH_ANONYMOUS=true
```

If `PREVIA_AUTH_ANONYMOUS` is unset, Previa treats it as `true`.

In anonymous mode:

- no login is required
- every protected route is available as the `anonymous` role
- `POST /api/v1/auth/login` returns a conflict because login is unnecessary
- JWTs and API tokens are not required

To explicitly keep a CLI-managed context open:

```bash
previa up -d --anonymous
```

### Protected

Protected mode starts when anonymous access is disabled:

```env
PREVIA_AUTH_ANONYMOUS=false
PREVIA_ROOT_USERNAME=root
PREVIA_ROOT_PASSWORD=change-me
PREVIA_JWT_SECRET=<long-random-secret>
```

When protected mode is active, every route requires authentication except:

- `GET /health`
- `POST /api/v1/auth/login`
- static app assets needed to render the login screen

For CLI-managed contexts, use:

```bash
printf '%s' 'change-me' | previa up -d --protected --root-username root --root-password-stdin
```

`previa up --protected` persists the access settings in the selected context
`main.env`:

```env
PREVIA_AUTH_ANONYMOUS=false
PREVIA_ROOT_USERNAME=root
PREVIA_ROOT_PASSWORD=change-me
PREVIA_JWT_SECRET=<generated-if-missing>
```

For direct `previa-main` startup, provide the environment yourself:

```bash
PREVIA_AUTH_ANONYMOUS=false \
PREVIA_ROOT_USERNAME=root \
PREVIA_ROOT_PASSWORD=change-me \
PREVIA_JWT_SECRET="$(uuidgen)" \
previa-main
```

Optional JWT TTL:

```env
PREVIA_JWT_TTL_SECONDS=86400
```

If omitted or invalid, JWTs default to 24 hours.

## Root Account

`PREVIA_ROOT_USERNAME` and `PREVIA_ROOT_PASSWORD` define the environment root
account. This account is not stored in the users table, so it is always
available as long as the protected-mode environment is present.

Use the root account to bootstrap the first database users and API tokens.
After that, prefer named users and named API tokens for day-to-day work.

## Login and API Tokens

Browser users log in through the app and receive a JWT. The app stores and
refreshes its own session state; direct API clients should not rely on app JWTs.

CLI users should store an API token:

```bash
printf '%s' 'change-me' | previa login --context default --username root --password-stdin
previa whoami --context default
```

`previa login` sends `clientKind=api_token` to the login API and stores the
returned fixed token in the selected context. For a URL that is not represented
by a local context, use:

```bash
printf '%s' 'change-me' | previa login --url https://previa.example.com --username root --password-stdin
```

Create named fixed tokens for automation:

```bash
previa token create --context default --name ci --role operator
previa token list --context default
previa token revoke --context default <token-id>
```

The raw API token is shown only when it is created. Store it in your secret
manager or environment at that moment.

For scripts, set:

```bash
export PREVIA_API_TOKEN='pvk_...'
```

`PREVIA_API_TOKEN` takes precedence over a token saved by `previa login` or
`previa token use`.

To store a token from an environment variable into a context:

```bash
previa token use --context default --token-env PREVIA_API_TOKEN
```

To remove a locally stored token:

```bash
previa logout --context default
```

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

Users cannot change their own role. This prevents an authenticated user from
promoting or demoting their own access level through the API.

Role creation rules:

- `root` can create and assign `root`, `admin`, `editor`, `operator`, and
  `viewer`
- `admin` can create and assign `admin`, `editor`, `operator`, and `viewer`
- `editor`, `operator`, and `viewer` cannot manage users or API tokens
- use `anonymous` through anonymous mode, not as a named user or token role

## Access UI

Root and admin users see the access button in the app header. The `/access`
page can:

- create users
- activate, deactivate, and remove users
- create API tokens and display the raw token once
- activate, deactivate, and revoke API tokens

Use users for people who open the browser app. Use API tokens for automation,
including CLI, MCP, CI, scripts, and direct API calls.

## Direct API Usage

In protected mode, direct API calls need a bearer token:

```bash
curl -fsS http://127.0.0.1:5588/api/v1/auth/me \
  -H "Authorization: Bearer $PREVIA_API_TOKEN"
```

To create an app JWT manually:

```bash
curl -fsS http://127.0.0.1:5588/api/v1/auth/login \
  -H "content-type: application/json" \
  -d '{"username":"root","password":"change-me","clientKind":"app"}'
```

To create an API token manually through login:

```bash
curl -fsS http://127.0.0.1:5588/api/v1/auth/login \
  -H "content-type: application/json" \
  -d '{"username":"root","password":"change-me","clientKind":"api_token","tokenName":"ci"}'
```

## Operational Notes

- Keep `PREVIA_ROOT_PASSWORD`, `PREVIA_JWT_SECRET`, and raw API tokens out of
  source control.
- Prefer `--root-password-stdin` instead of putting the root password in shell
  history.
- Use the least-privileged API token role that can perform the job.
- Revoke unused API tokens from the Access UI or `previa token revoke`.
