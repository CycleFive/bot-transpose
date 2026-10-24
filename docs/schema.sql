-- The (platform, raw) column pair, in Postgres. An illustration for cracktunes'
-- migrations, not used by the proof of concept, which has no database.

-- The typed platform. Values match `Platform::as_str()` in crates/core/src/ids.rs.
CREATE TYPE platform AS ENUM ('discord', 'matrix', 'stoat', 'telegram');

-- Lifecycle, mirroring `Platform::lifecycle()`, so the database can refuse new
-- rows for a platform that is not active while old rows stay readable.
CREATE TABLE platform_lifecycle (
    platform  platform PRIMARY KEY,
    lifecycle text NOT NULL CHECK (lifecycle IN ('active', 'modelled', 'deprecated'))
);
INSERT INTO platform_lifecycle VALUES
    ('discord', 'active'), ('matrix', 'active'), ('stoat', 'active'), ('telegram', 'modelled');

-- Extending: ALTER TYPE platform ADD VALUE 'fluxer'; plus a lifecycle row and a
-- new `Platform` variant. Deprecating: UPDATE platform_lifecycle SET lifecycle =
-- 'deprecated' WHERE platform = '…'; plus `Lifecycle::Deprecated` in Rust. Enum
-- values are never dropped while rows mention them.

-- Settings: one row per scope. The scope is (kind, platform, raw).
CREATE TABLE settings (
    scope_kind     text     NOT NULL CHECK (scope_kind IN ('community', 'room')),
    scope_platform platform NOT NULL,
    scope_raw      text     NOT NULL CHECK (scope_raw <> ''),
    prefix         text     NOT NULL DEFAULT '!',
    volume         real     NOT NULL DEFAULT 0.5,
    autoplay       boolean  NOT NULL DEFAULT true,
    dj_gate        jsonb    NOT NULL DEFAULT '{"type":"anyone"}',
    admin_gate     jsonb    NOT NULL DEFAULT '{"type":"admin"}',
    PRIMARY KEY (scope_kind, scope_platform, scope_raw)
);

-- What GuildSettings keeps as bare u64 today, as typed pairs. A check keeps
-- every user on the scope's platform (Settings::check does the same in Rust).
CREATE TABLE settings_authorized_user (
    scope_kind     text     NOT NULL,
    scope_platform platform NOT NULL,
    scope_raw      text     NOT NULL,
    user_platform  platform NOT NULL,
    user_raw       text     NOT NULL CHECK (user_raw <> ''),
    PRIMARY KEY (scope_kind, scope_platform, scope_raw, user_platform, user_raw),
    FOREIGN KEY (scope_kind, scope_platform, scope_raw) REFERENCES settings ON DELETE CASCADE,
    CHECK (user_platform = scope_platform)
);

CREATE TABLE settings_ignored_room (
    scope_kind     text     NOT NULL,
    scope_platform platform NOT NULL,
    scope_raw      text     NOT NULL,
    room_platform  platform NOT NULL,
    room_raw       text     NOT NULL CHECK (room_raw <> ''),
    PRIMARY KEY (scope_kind, scope_platform, scope_raw, room_platform, room_raw),
    FOREIGN KEY (scope_kind, scope_platform, scope_raw) REFERENCES settings ON DELETE CASCADE,
    CHECK (room_platform = scope_platform)
);

-- New rows only for active platforms.
CREATE FUNCTION refuse_inactive_platform() RETURNS trigger AS $$
BEGIN
    IF (SELECT lifecycle FROM platform_lifecycle WHERE platform = NEW.scope_platform) <> 'active' THEN
        RAISE EXCEPTION 'platform % takes no new rows', NEW.scope_platform;
    END IF;
    RETURN NEW;
END $$ LANGUAGE plpgsql;

CREATE TRIGGER settings_active_platform BEFORE INSERT ON settings
    FOR EACH ROW EXECUTE FUNCTION refuse_inactive_platform();
