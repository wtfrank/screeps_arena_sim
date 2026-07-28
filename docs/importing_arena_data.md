# Importing Arena Data

The simulator needs real map data (terrain and initial object layouts) captured from the live
Screeps Arena game client. This is done by routing the game client's HTTPS traffic through
[mitmproxy](https://mitmproxy.org/), saving the relevant API responses to a `.flows` file, and
then running the import tools to populate the local databases.

## Prerequisites

- mitmproxy installed (`pip install mitmproxy` or system package)
- Screeps Arena installed via Steam
- `libnss3-tools` installed for `certutil` (`sudo apt install libnss3-tools`)

---

## One-time Setup: Trust the mitmproxy CA Certificate

Screeps Arena is an Electron (Chromium-based) app. Chromium on Linux uses the **NSS certificate
database** (`~/.pki/nssdb`) rather than the system CA store, so `update-ca-certificates` alone
is not sufficient.

### 1. Start mitmproxy once to generate its CA certificate

```bash
mitmproxy
# Ctrl-C to quit immediately — we only need the cert generated
```

The CA certificate is written to `~/.mitmproxy/mitmproxy-ca-cert.pem`.

### 2. Create the NSS database if it does not already exist

```bash
mkdir -p ~/.pki/nssdb
certutil -d sql:~/.pki/nssdb -N --empty-password
```

### 3. Import the mitmproxy CA certificate into the NSS database

```bash
certutil -d sql:~/.pki/nssdb -A -t "CT,C,C" -n mitmproxy \
    -i ~/.mitmproxy/mitmproxy-ca-cert.pem
```

### 4. Verify the certificate was imported

```bash
certutil -d sql:~/.pki/nssdb -L
```

You should see `mitmproxy` in the list with trust bits `CT,C,C`.

---

## Steam Launch Options

Configure Screeps Arena to route its traffic through the local mitmproxy instance.

In Steam: right-click **Screeps Arena** → **Properties** → **Launch Options**, and set:

```
HTTPS_PROXY=http://127.0.0.1:8080 HTTP_PROXY=http://127.0.0.1:8080 %command% --proxy-server="http://127.0.0.1:8080" --no-sandbox
```

> **Note:** Remove or disable these launch options after capturing data. Playing through the
> proxy is unnecessary once you have the flows you need.

---

## Clear the Electron Cache Before Capturing

> **This step is crucial.** Chromium caches API responses aggressively. Without clearing the
> cache first, most requests will return HTTP 304 Not Modified with no response body, making
> them invisible to mitmproxy and the import tools.

The cache lives in the Screeps Arena Electron user data directory:

```bash
rm -rf ~/.config/screeps_arena/Cache
rm -rf ~/.config/screeps_arena/'Code Cache'
rm -rf ~/.config/screeps_arena/'Session Storage'
rm -rf ~/.config/screeps_arena/'Local Storage'
```

`Cookies` is left intact so you remain logged in. This must be done **before** launching the
game each time you want a clean capture. The 304 responses occur because Chromium sends
`If-None-Match` / `If-Modified-Since` headers built from previously cached ETags — clearing
the cache removes those stored ETags so the server returns full 200 responses.

---

## Capturing a Session

### 1. Start mitmproxy and save all flows to a file

Start mitmproxy **before** launching the game so that the initial API calls are captured
from the very beginning of the session:

```bash
mitmproxy -w ~/screeps_arena_session.flows
```

The `-w` flag writes every intercepted flow to the file in real time.

### 2. Launch Screeps Arena via Steam

Open the game normally and log in.

#### Capturing season and arena metadata

The game automatically calls `/api/season/list`, `/api/season/current`, and
`/api/season/<id>/arenas` on startup. These are captured without any special interaction.
However, to also capture individual `/api/arena/<id>` responses (which include arena
descriptions), **click on each of the 6 arenas** in the lobby to open their detail panel.
You do **not** need to have unlocked an arena for this.

#### Capturing map layout data (terrain and objects)

To capture terrain and initial object positions you must **enter and play a match** in that
arena. This requires the arena to be **unlocked**. Play several matches per arena to
accumulate multiple layout seeds.

### 3. Stop mitmproxy

Press `q` in the mitmproxy UI to quit once you have captured what you need.

---

## Importing Seasons and Arenas

The `tools/import_arenas.py` script scans the flow file for successful responses from the
season API endpoints and upserts them into the local database at
`~/.local/share/screeps_arena_sim/arenas.json` (respecting `$XDG_DATA_HOME`).

```bash
python3 screeps_arena_sim/tools/import_arenas.py ~/screeps_arena_session.flows
```

It extracts data from three endpoints:
- `/api/season/list` — all seasons (historical and current)
- `/api/season/current` — which season is currently active
- `/api/season/<id>/arenas` — all arenas belonging to each season

Running the importer multiple times against the same or different flow files is safe —
existing entries are updated in place and duplicates are not added.

### Optional: specify a custom database path

```bash
python3 screeps_arena_sim/tools/import_arenas.py ~/screeps_arena_session.flows \
    --db /path/to/custom/arenas.json
```

### Example output

```
Seasons: 5 added, 0 updated
Arenas:  30 added, 0 updated
Database: /home/user/.local/share/screeps_arena_sim/arenas.json
Current season: Season 3 (69cfdf57ece2ae9f75da12cc)

Season Alpha  (61e089e1ba623e0df9ccc506)
  Capture the Flag  v52  folder=alpha-capture_the_flag  (606873c364da921cb49855f7)
  ...

Season 3 [CURRENT]  (69cfdf57ece2ae9f75da12cc)
  Spawn Strike 3  folder=season3-spawn_strike  (69cfe6fcece2ae9f75da12d1)
  Spawn Strike 3 [advanced]  folder=season3-spawn_strike-advanced  (69cfe6fcece2ae9f75da12d2)
  ...
```

---

## Inspecting Captured Flows

```bash
# Open saved flows in the interactive browser
mitmproxy -r ~/screeps_arena_session.flows

# Filter to only JSON API responses
mitmdump -r ~/screeps_arena_session.flows -f '~t application/json'
```

To export a single response body from within the mitmproxy UI:
1. Select the flow and press `Enter`
2. Press `Tab` to switch to the Response tab
3. Press `:` and run: `export.file raw_response @focus /path/to/output.json`

---

## Next Steps

- **Import map layouts**: run `tools/import_layouts.py` (not yet implemented) against the same
  flow file to populate terrain and initial object data for each arena.
- **Capture multiple seeds**: play several matches of the same arena and re-run the importer to
  accumulate a library of layout variants for more representative simulation testing.
