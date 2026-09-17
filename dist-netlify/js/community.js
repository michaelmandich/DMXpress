// DMXexpress community board: Google sign-in, usernames, threads, comments,
// screenshots, and delete-your-own. All permissions are enforced by Supabase
// Row Level Security - this file is just the view.

(function () {
  const esc = window.dmxEsc;
  const ago = window.dmxAgo;
  const $ = (id) => document.getElementById(id);

  const CATS = [
    ["", "All"],
    ["general", "General"],
    ["help", "Help"],
    ["shows", "Show files"],
    ["plugins", "Plugins"],
    ["bugs", "Bugs"],
  ];
  const CAT_LABEL = Object.fromEntries(CATS);

  let me = null; // { id, username } when signed in with a profile
  let session = null;
  let category = "";

  const client = window.dmxClient();

  function notice(text) {
    const el = $("forum-status");
    el.hidden = !text;
    el.textContent = text || "";
  }

  // ---------- auth ----------
  async function refreshAuth() {
    const row = $("auth-row");
    if (!client) {
      notice(
        "The board is not connected yet (Supabase keys pending in js/config.js). " +
          "In the meantime, use GitHub issues for bugs and discussions."
      );
      row.innerHTML = "";
      $("new-thread-btn").hidden = true;
      return;
    }
    const { data } = await client.auth.getSession();
    session = data ? data.session : null;
    if (!session) {
      row.innerHTML = '<button class="btn btn-primary" id="signin">Sign in with Google</button><span class="fine">Read without signing in; post with an account.</span>';
      $("signin").addEventListener("click", () =>
        client.auth.signInWithOAuth({
          provider: "google",
          options: { redirectTo: window.location.href.split("#")[0] },
        })
      );
      me = null;
      $("username-box").hidden = true;
      $("new-thread-btn").hidden = true;
      return;
    }
    const { data: profile } = await client
      .from("profiles")
      .select("id,username")
      .eq("id", session.user.id)
      .maybeSingle();
    if (!profile) {
      me = null;
      row.innerHTML = "";
      $("username-box").hidden = false;
      $("new-thread-btn").hidden = true;
      return;
    }
    me = profile;
    $("username-box").hidden = true;
    $("new-thread-btn").hidden = false;
    row.innerHTML = `<span class="fine">Signed in as <strong>@${esc(me.username)}</strong></span> <a href="#" id="signout" class="fine">sign out</a>`;
    $("signout").addEventListener("click", async (e) => {
      e.preventDefault();
      await client.auth.signOut();
      refreshAuth();
    });
  }

  $("username-save").addEventListener("click", async () => {
    const name = $("username-input").value.trim();
    const msg = $("username-msg");
    if (!/^[A-Za-z0-9_\-]{3,24}$/.test(name)) {
      msg.textContent = "3-24 letters, numbers, dashes or underscores.";
      return;
    }
    msg.textContent = "Saving...";
    const { error } = await client.from("profiles").insert({ id: session.user.id, username: name });
    if (error) {
      msg.textContent = error.code === "23505" ? "That username is taken." : "Failed: " + error.message;
    } else {
      msg.textContent = "";
      await refreshAuth();
      loadList();
    }
  });

  // ---------- screenshots ----------
  async function uploadImage(file) {
    if (!file) return null;
    if (file.size > 4 * 1024 * 1024) throw new Error("Image too large (4 MB max).");
    const safe = file.name.replace(/[^A-Za-z0-9_.\-]/g, "_").slice(-60);
    const path = `${session.user.id}/${Date.now()}-${safe}`;
    const { error } = await client.storage.from("forum-images").upload(path, file, {
      cacheControl: "31536000",
      contentType: file.type,
    });
    if (error) throw error;
    return client.storage.from("forum-images").getPublicUrl(path).data.publicUrl;
  }

  const imageTag = (url) =>
    url
      ? `<a href="${esc(url)}" target="_blank" rel="noopener"><img class="post-shot" src="${esc(url)}" alt="attached screenshot" loading="lazy"></a>`
      : "";

  // ---------- thread list ----------
  function chipRow() {
    $("cat-chips").innerHTML = CATS.map(
      ([v, label]) =>
        `<button class="chip${v === category ? " is-on" : ""}" data-cat="${v}">${label}</button>`
    ).join("");
    $("cat-chips")
      .querySelectorAll(".chip")
      .forEach((c) =>
        c.addEventListener("click", () => {
          category = c.dataset.cat;
          chipRow();
          loadList();
        })
      );
  }

  async function loadList() {
    if (!client) return;
    let q = client
      .from("threads")
      .select("id,title,category,created_at,last_post_at,reply_count,image_url,author,profiles(username)")
      .order("last_post_at", { ascending: false })
      .limit(100);
    if (category) q = q.eq("category", category);
    const { data, error } = await q;
    const list = $("thread-list");
    if (error) {
      notice("Could not load threads: " + error.message);
      list.innerHTML = "";
      return;
    }
    notice("");
    if (!data.length) {
      list.innerHTML = '<p class="fine" style="padding:24px 4px">Nothing here yet. Start the first thread.</p>';
      return;
    }
    list.innerHTML = data
      .map(
        (t) => `<a class="thread-row" href="#t/${t.id}">
          <div class="thread-main">
            <span class="thread-title">${esc(t.title)}</span>
            <span class="fine">by @${esc(t.profiles ? t.profiles.username : "unknown")} · ${ago(t.created_at)}${t.image_url ? " · has screenshot" : ""}</span>
          </div>
          <span class="chip is-static">${esc(CAT_LABEL[t.category] || t.category)}</span>
          <span class="thread-replies">${t.reply_count}</span>
        </a>`
      )
      .join("");
  }

  // ---------- thread view ----------
  async function loadThread(id) {
    if (!client) return;
    const view = $("thread-view");
    $("board").hidden = true;
    view.hidden = false;
    view.innerHTML = '<p class="fine">Loading thread...</p>';
    const { data: t, error } = await client
      .from("threads")
      .select("id,title,body,category,created_at,image_url,author,profiles(username)")
      .eq("id", id)
      .maybeSingle();
    if (error || !t) {
      view.innerHTML = '<p class="fine">Thread not found. <a href="#">Back to the board</a></p>';
      return;
    }
    const { data: posts } = await client
      .from("posts")
      .select("id,body,created_at,image_url,author,profiles(username)")
      .eq("thread_id", id)
      .order("created_at", { ascending: true });

    const mine = (author) => me && me.id === author;
    const del = (kind, pid, author) =>
      mine(author) ? `<button class="link-danger" data-del="${kind}:${pid}">delete</button>` : "";

    view.innerHTML = `
      <p><a href="#">&larr; Back to the board</a></p>
      <article class="thread-card">
        <div class="thread-head">
          <h2>${esc(t.title)}</h2>
          <span class="chip is-static">${esc(CAT_LABEL[t.category] || t.category)}</span>
        </div>
        <p class="fine">by @${esc(t.profiles ? t.profiles.username : "unknown")} · ${ago(t.created_at)} ${del("thread", t.id, t.author)}</p>
        <p class="post-body">${esc(t.body)}</p>
        ${imageTag(t.image_url)}
      </article>
      <div id="posts">
        ${(posts || [])
          .map(
            (p) => `<article class="post-card">
              <p class="fine">@${esc(p.profiles ? p.profiles.username : "unknown")} · ${ago(p.created_at)} ${del("post", p.id, p.author)}</p>
              <p class="post-body">${esc(p.body)}</p>
              ${imageTag(p.image_url)}
            </article>`
          )
          .join("")}
      </div>
      ${
        me
          ? `<div class="form-panel">
              <label class="field field-wide"><span>Reply as @${esc(me.username)}</span>
                <textarea id="reply-body" rows="3" maxlength="10000"></textarea>
              </label>
              <div class="form-actions">
                <input id="reply-image" type="file" accept="image/png,image/jpeg,image/webp">
                <button class="btn btn-primary" id="reply-send">Reply</button>
                <span class="fine" id="reply-msg"></span>
              </div>
            </div>`
          : '<p class="fine">Sign in at the top to reply.</p>'
      }`;

    view.querySelectorAll("[data-del]").forEach((b) =>
      b.addEventListener("click", async () => {
        const [kind, pid] = b.dataset.del.split(":");
        if (!window.confirm("Delete this " + (kind === "thread" ? "thread and all its comments?" : "comment?"))) return;
        const table = kind === "thread" ? "threads" : "posts";
        const { error: e } = await client.from(table).delete().eq("id", pid);
        if (e) {
          alert("Delete failed: " + e.message);
        } else if (kind === "thread") {
          window.location.hash = "";
        } else {
          loadThread(id);
        }
      })
    );

    const send = $("reply-send");
    if (send) {
      send.addEventListener("click", async () => {
        const body = $("reply-body").value.trim();
        const msg = $("reply-msg");
        if (!body) return;
        msg.textContent = "Posting...";
        try {
          const image_url = await uploadImage($("reply-image").files[0]);
          const { error: e } = await client.from("posts").insert({
            thread_id: Number(id),
            author: me.id,
            body,
            image_url,
          });
          if (e) throw e;
          loadThread(id);
        } catch (e) {
          msg.textContent = "Failed: " + e.message;
        }
      });
    }
  }

  // ---------- new thread ----------
  $("new-thread-btn").addEventListener("click", () => {
    $("new-thread-form").hidden = false;
    $("new-thread-btn").hidden = true;
  });
  $("nt-cancel").addEventListener("click", () => {
    $("new-thread-form").hidden = true;
    $("new-thread-btn").hidden = !me;
  });
  $("nt-submit").addEventListener("click", async () => {
    const title = $("nt-title").value.trim();
    const body = $("nt-body").value.trim();
    const msg = $("nt-msg");
    if (title.length < 3 || !body) {
      msg.textContent = "A title (3+ characters) and a post are required.";
      return;
    }
    msg.textContent = "Posting...";
    try {
      const image_url = await uploadImage($("nt-image").files[0]);
      const { data, error } = await client
        .from("threads")
        .insert({ author: me.id, title, body, category: $("nt-category").value, image_url })
        .select("id")
        .single();
      if (error) throw error;
      $("nt-title").value = "";
      $("nt-body").value = "";
      $("nt-image").value = "";
      $("new-thread-form").hidden = true;
      $("new-thread-btn").hidden = false;
      window.location.hash = "#t/" + data.id;
    } catch (e) {
      msg.textContent = "Failed: " + e.message;
    }
  });

  // ---------- routing ----------
  function route() {
    const m = window.location.hash.match(/^#t\/(\d+)$/);
    if (m && client) {
      loadThread(m[1]);
    } else {
      $("thread-view").hidden = true;
      $("board").hidden = false;
      loadList();
    }
  }
  window.addEventListener("hashchange", route);

  // Supabase appends tokens in the URL after OAuth; its client stores the
  // session automatically, so a refresh here just re-reads it.
  if (client) {
    client.auth.onAuthStateChange(() => refreshAuth());
  }

  chipRow();
  refreshAuth().then(route);
})();
