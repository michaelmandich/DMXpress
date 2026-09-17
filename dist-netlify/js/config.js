// Supabase wiring for the DMXexpress site (community + plugin registry).
//
// 1. Paste your project URL and anon key below. The anon (public) key is
//    designed to ship in client-side code - Row Level Security enforces all
//    permissions server-side. NEVER put the service_role key here.
// 2. Run supabase/schema.sql (in the repo) once in the Supabase SQL editor.
// 3. Enable the Google provider under Authentication -> Providers.
window.DMX_SUPABASE = {
  url: "https://nizfsocllvpauzksuzpq.supabase.co",
  anonKey: "sb_publishable_ddiFZHbGirHW3QUKfX94Xw_d8tgGqp1",
};

window.dmxSupabaseReady = function () {
  const c = window.DMX_SUPABASE;
  return (
    c &&
    /^https:\/\//.test(c.url) &&
    c.anonKey &&
    c.anonKey.length > 40 &&
    typeof window.supabase !== "undefined"
  );
};

window.dmxClient = function () {
  if (!window._dmxClient && window.dmxSupabaseReady()) {
    window._dmxClient = window.supabase.createClient(
      window.DMX_SUPABASE.url,
      window.DMX_SUPABASE.anonKey
    );
  }
  return window._dmxClient || null;
};

// Escape user text before injecting into HTML.
window.dmxEsc = function (s) {
  return String(s == null ? "" : s).replace(
    /[&<>"']/g,
    (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[ch])
  );
};

window.dmxAgo = function (iso) {
  const s = (Date.now() - new Date(iso).getTime()) / 1000;
  if (s < 60) return "just now";
  if (s < 3600) return Math.floor(s / 60) + "m ago";
  if (s < 86400) return Math.floor(s / 3600) + "h ago";
  if (s < 2592000) return Math.floor(s / 86400) + "d ago";
  return new Date(iso).toLocaleDateString();
};
