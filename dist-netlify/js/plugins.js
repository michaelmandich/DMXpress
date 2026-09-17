// Plugin registry: bundled starters always show; approved community plugins
// load from Supabase when js/config.js is filled in.

(function () {
  const grid = document.getElementById("plugin-grid");
  const status = document.getElementById("registry-status");
  const esc = window.dmxEsc;

  const BUNDLED = [
    {
      id: "breathing-floor",
      name: "Breathing Floor",
      version: "1.0",
      author_name: "DMXpress",
      category: "effect",
      description:
        "A slow sine breath over every dimmer, fanned across the stage left to right. Rate, depth, and floor level are sliders.",
      download_url: "plugins/breathing_floor.rhai",
      bundled: true,
    },
    {
      id: "strobe-burst",
      name: "Strobe Burst",
      version: "1.0",
      author_name: "DMXpress",
      category: "effect",
      description:
        "A one-shot white flash over the whole rig that decays back out. Mash the BURST button with the kick drum.",
      download_url: "plugins/strobe_burst.rhai",
      bundled: true,
    },
    {
      id: "neon-night",
      name: "Neon Night",
      version: "1.0",
      author_name: "DMXpress",
      category: "look",
      description:
        "Re-tints the whole console hot pink for late-set energy. A theme-only plugin: no layer, no window, just a look.",
      download_url: "plugins/neon_night.rhai",
      bundled: true,
    },
  ];

  const CATEGORY = { effect: "Effect", tool: "Tool", look: "Look" };

  function card(p) {
    const dl = p.downloads ? `<span class="fine">${p.downloads} downloads</span>` : "";
    const home = p.homepage
      ? ` <a href="${esc(p.homepage)}" target="_blank" rel="noopener" class="fine">source</a>`
      : "";
    const badge = p.bundled
      ? '<span class="badge-new">Built in</span>'
      : '<span class="badge-new">Community</span>';
    return `<div class="card">
      <div class="card-icon">${esc((CATEGORY[p.category] || "FX").slice(0, 2).toUpperCase())}</div>
      <h3>${esc(p.name)} ${badge}</h3>
      <p class="fine">v${esc(p.version)} by ${esc(p.author_name || "unknown")} · ${esc(CATEGORY[p.category] || p.category)}${home}</p>
      <p>${esc(p.description)}</p>
      <p style="margin-top:16px">
        <a class="btn btn-primary" href="${esc(p.download_url)}" download data-plugin="${esc(p.id)}">Download .rhai</a>
        ${dl}
      </p>
    </div>`;
  }

  function render(list) {
    grid.innerHTML = list.map(card).join("");
    grid.querySelectorAll("a[data-plugin]").forEach((a) => {
      a.addEventListener("click", () => {
        const client = window.dmxClient();
        if (client && !BUNDLED.some((b) => b.id === a.dataset.plugin)) {
          client.rpc("bump_download", { plugin_id: a.dataset.plugin }).then(() => {});
        }
      });
    });
  }

  async function load() {
    let list = [...BUNDLED];
    const client = window.dmxClient();
    if (!client) {
      status.hidden = false;
      status.textContent =
        "Community registry is not connected yet (Supabase keys pending) - showing the built-in plugins.";
      render(list);
      return;
    }
    const { data, error } = await client
      .from("plugins")
      .select("id,name,version,author_name,description,category,download_url,homepage,downloads")
      .eq("approved", true)
      .order("created_at", { ascending: false });
    if (error) {
      status.hidden = false;
      status.textContent = "Could not reach the registry: " + error.message;
    } else if (data) {
      list = [...data, ...list];
    }
    render(list);
  }

  // ---------- submission form ----------
  const authRow = document.getElementById("submit-auth");
  const form = document.getElementById("submit-form");
  const msg = document.getElementById("submit-msg");

  async function refreshAuth() {
    const client = window.dmxClient();
    if (!client) {
      authRow.innerHTML =
        '<span class="fine">Submissions open once the registry is connected. For now, share your plugin as a GitHub link on the <a href="community.html">forum</a>.</span>';
      form.hidden = true;
      return;
    }
    const { data } = await client.auth.getSession();
    const session = data ? data.session : null;
    if (!session) {
      authRow.innerHTML = '<button class="btn btn-primary" id="pl-signin">Sign in with Google to submit</button>';
      form.hidden = true;
      document.getElementById("pl-signin").addEventListener("click", () => {
        client.auth.signInWithOAuth({
          provider: "google",
          options: { redirectTo: window.location.href.split("#")[0] },
        });
      });
      return;
    }
    const { data: profile } = await client
      .from("profiles")
      .select("username")
      .eq("id", session.user.id)
      .maybeSingle();
    if (!profile) {
      authRow.innerHTML =
        '<span class="fine">Almost there - pick a username on the <a href="community.html">community page</a> first, then come back to submit.</span>';
      form.hidden = true;
      return;
    }
    authRow.innerHTML = `<span class="fine">Submitting as <strong>@${esc(profile.username)}</strong> · <a href="#" id="pl-signout">sign out</a></span>`;
    form.hidden = false;
    document.getElementById("pl-signout").addEventListener("click", async (e) => {
      e.preventDefault();
      await client.auth.signOut();
      refreshAuth();
    });
  }

  if (form) {
    form.addEventListener("submit", async (e) => {
      e.preventDefault();
      const client = window.dmxClient();
      if (!client) return;
      const { data } = await client.auth.getSession();
      if (!data || !data.session) return;
      const f = new FormData(form);
      const { data: profile } = await client
        .from("profiles")
        .select("username")
        .eq("id", data.session.user.id)
        .maybeSingle();
      msg.textContent = "Submitting...";
      const { error } = await client.from("plugins").insert({
        id: f.get("id"),
        name: f.get("name"),
        version: f.get("version") || "1.0",
        category: f.get("category"),
        description: f.get("description") || "",
        download_url: f.get("download_url"),
        homepage: f.get("homepage") || null,
        author_name: profile ? profile.username : "unknown",
        submitted_by: data.session.user.id,
        approved: false,
      });
      if (error) {
        msg.textContent = "Submission failed: " + error.message;
      } else {
        msg.textContent = "Submitted - it will appear once approved. Thanks!";
        form.reset();
      }
    });
  }

  load();
  refreshAuth();
})();
