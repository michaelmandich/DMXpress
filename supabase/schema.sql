-- ============================================================
-- DMXexpress community + plugin registry - Supabase schema
-- ============================================================
-- Run this once in your Supabase project: SQL Editor -> New query -> paste -> Run.
--
-- Manual dashboard steps (one-time):
--  1. Authentication -> Providers -> Google -> Enable.
--     Create OAuth credentials at console.cloud.google.com (OAuth client,
--     type "Web application"), set the redirect URL Supabase shows you,
--     then paste the client ID and secret into Supabase.
--  2. Authentication -> URL Configuration -> add your Netlify site URL
--     (and http://localhost for testing) to "Redirect URLs".
--  3. To get an email when a plugin is submitted: Database -> Webhooks ->
--     Create webhook on table public.plugins, event INSERT, and point it at
--     an email service (Zapier/Make/Resend all work), or just check the
--     pending queue: select * from plugins where approved = false;
--  4. Approving a plugin: Table Editor -> plugins -> set approved = true.
-- ============================================================

-- ---------- profiles: the username people post under ----------
create table if not exists public.profiles (
  id         uuid primary key references auth.users (id) on delete cascade,
  username   text not null unique
             check (username ~ '^[A-Za-z0-9_\-]{3,24}$'),
  created_at timestamptz not null default now()
);

alter table public.profiles enable row level security;

create policy "profiles: anyone can read"
  on public.profiles for select using (true);

create policy "profiles: create your own"
  on public.profiles for insert to authenticated
  with check (auth.uid() = id);

create policy "profiles: update your own"
  on public.profiles for update to authenticated
  using (auth.uid() = id) with check (auth.uid() = id);

-- ---------- forum threads ----------
create table if not exists public.threads (
  id           bigint generated always as identity primary key,
  author       uuid not null references public.profiles (id) on delete cascade,
  category     text not null default 'general'
               check (category in ('general', 'help', 'shows', 'plugins', 'bugs')),
  title        text not null check (char_length(title) between 3 and 120),
  body         text not null check (char_length(body) between 1 and 10000),
  image_url    text,
  reply_count  integer not null default 0,
  created_at   timestamptz not null default now(),
  last_post_at timestamptz not null default now()
);

alter table public.threads enable row level security;

create policy "threads: anyone can read"
  on public.threads for select using (true);

create policy "threads: signed-in users create as themselves"
  on public.threads for insert to authenticated
  with check (auth.uid() = author);

create policy "threads: delete your own"
  on public.threads for delete to authenticated
  using (auth.uid() = author);

create index if not exists threads_activity on public.threads (last_post_at desc);

-- ---------- forum comments ----------
create table if not exists public.posts (
  id         bigint generated always as identity primary key,
  thread_id  bigint not null references public.threads (id) on delete cascade,
  author     uuid not null references public.profiles (id) on delete cascade,
  body       text not null check (char_length(body) between 1 and 10000),
  image_url  text,
  created_at timestamptz not null default now()
);

alter table public.posts enable row level security;

create policy "posts: anyone can read"
  on public.posts for select using (true);

create policy "posts: signed-in users create as themselves"
  on public.posts for insert to authenticated
  with check (auth.uid() = author);

create policy "posts: delete your own"
  on public.posts for delete to authenticated
  using (auth.uid() = author);

create index if not exists posts_thread on public.posts (thread_id, created_at);

-- Keep the thread's reply count and activity timestamp fresh.
create or replace function public.bump_thread()
returns trigger
language plpgsql security definer set search_path = public
as $$
begin
  if tg_op = 'INSERT' then
    update public.threads
      set reply_count = reply_count + 1, last_post_at = now()
      where id = new.thread_id;
    return new;
  elsif tg_op = 'DELETE' then
    update public.threads
      set reply_count = greatest(reply_count - 1, 0)
      where id = old.thread_id;
    return old;
  end if;
  return null;
end $$;

drop trigger if exists posts_bump on public.posts;
create trigger posts_bump
  after insert or delete on public.posts
  for each row execute function public.bump_thread();

-- ---------- plugin registry ----------
-- Anyone signed in can SUBMIT (approved is forced false); only rows with
-- approved = true are visible to the site and the app. You approve from the
-- dashboard (Table Editor -> plugins -> approved = true).
create table if not exists public.plugins (
  id           text primary key
               check (id ~ '^[a-z0-9][a-z0-9\-]{1,48}$'),
  name         text not null check (char_length(name) between 2 and 60),
  version      text not null default '1.0',
  author_name  text not null default '',
  description  text not null default '' check (char_length(description) <= 500),
  category     text not null default 'effect'
               check (category in ('effect', 'look', 'tool')),
  -- A direct link to the .rhai file: a GitHub raw/release URL works great.
  download_url text not null check (download_url ~ '^https://'),
  homepage     text check (homepage is null or homepage ~ '^https://'),
  approved     boolean not null default false,
  submitted_by uuid references public.profiles (id) on delete set null,
  downloads    bigint not null default 0,
  created_at   timestamptz not null default now()
);

alter table public.plugins enable row level security;

create policy "plugins: approved are public"
  on public.plugins for select using (approved);

create policy "plugins: signed-in users submit unapproved"
  on public.plugins for insert to authenticated
  with check (auth.uid() = submitted_by and approved = false);

create policy "plugins: submitters may withdraw unapproved"
  on public.plugins for delete to authenticated
  using (auth.uid() = submitted_by and not approved);

-- Public download counter (called by the site's Download buttons).
create or replace function public.bump_download(plugin_id text)
returns void
language sql security definer set search_path = public
as $$
  update public.plugins set downloads = downloads + 1
    where id = plugin_id and approved;
$$;

grant execute on function public.bump_download(text) to anon, authenticated;

-- ---------- screenshots bucket ----------
insert into storage.buckets (id, name, public)
values ('forum-images', 'forum-images', true)
on conflict (id) do nothing;

create policy "forum images: anyone can view"
  on storage.objects for select
  using (bucket_id = 'forum-images');

-- Uploads land in a folder named after the uploader's user id.
create policy "forum images: signed-in upload to own folder"
  on storage.objects for insert to authenticated
  with check (
    bucket_id = 'forum-images'
    and (storage.foldername(name))[1] = auth.uid()::text
  );

create policy "forum images: delete your own"
  on storage.objects for delete to authenticated
  using (bucket_id = 'forum-images' and owner = auth.uid());
