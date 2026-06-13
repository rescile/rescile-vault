import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { FormEvent, ChangeEvent } from 'react'

type Config = {
  url: string
  clientname: string
  has_url: boolean
  has_clientname: boolean
  has_password: boolean
  ready: boolean
}

type CollectionInfo = {
  name: string
  has_access: boolean
  pending_invite: boolean
  members: string[]
  pending_invitees: string[]
}

type State = {
  clientname: string
  collections: CollectionInfo[]
}

type Message = { kind: 'ok' | 'err'; text: string }

type SecretRow = {
  id: string
  name: string
  value: string
  revealed: boolean
  busy: boolean
  message: Message | null
}

type IssuedInvite = {
  id: string
  client: string
  token: string
  validity?: string
}

async function api<T = unknown>(
  path: string,
  init?: RequestInit,
): Promise<{ ok: boolean; status: number; data: T & { error?: string } }> {
  const res = await fetch(path, {
    ...(init || {}),
    headers: {
      'Content-Type': 'application/json',
      ...(init?.headers || {}),
    },
  })
  let data: unknown = {}
  try {
    data = await res.json()
  } catch {
    data = {}
  }
  return { ok: res.ok, status: res.status, data: data as T & { error?: string } }
}

function newId(): string {
  return `id-${Math.random().toString(36).slice(2, 10)}-${Date.now().toString(36)}`
}

function useTheme() {
  const [theme, setTheme] = useState(() => {
    const saved = localStorage.getItem('theme')
    if (saved) return saved
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
  })
  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme)
    localStorage.setItem('theme', theme)
  }, [theme])
  const toggle = () => setTheme(t => (t === 'dark' ? 'light' : 'dark'))
  return { theme, toggle }
}

function Header({
  theme,
  onToggleTheme,
  clientname,
  onLogout,
}: {
  theme: string
  onToggleTheme: () => void
  clientname?: string
  onLogout?: () => void
}) {
  return (
    <div className="header-container">
      <h1>
        <a href="/">
          <svg
            style={{ height: '1.4em', width: 'auto' }}
            viewBox="0 0 211 163"
            xmlns="http://www.w3.org/2000/svg"
          >
            <circle cx="103" cy="81" r="81" fill="var(--primary)" />
            <path
              d="M72.7 8.1c8.4-.5 16.8.8 24.9 3 5.3 1.8 10 4.2 13.2 8.2 2.8 4.1 2.2 5.1 1.5 10.5l13.2 60.7c3.9 18.8 12.6 21.4 20.5 25.7 7 4.5 13.2 7.4 24.3 10.3-7.6 2.3-15.3 4.5-24.3 5.4-5.5.1-11.1.8-15.7-1.4-10.5-5.5-13.3-12.9-16.4-20.2-4.6-11.9-5.7-23.6-7.8-35.2C101.2 89.5 92.6 98.8 84.8 108.9c-6.1 7.5-12.2 13-18.4 16.9-4.2 2.7-5.5 3.5-9.4 5.2-4.1 1.4-7.8 3.1-14.2 3.8 4.4-3.4 8.8-6.4 13.1-10.8 2.6-2.3 7-7.6 15.4-19.4 6.1-8.6 10.5-16.8 14.6-24.6 5.2-10.6 7.2-16.8 9-24.9.1-3.4-.7-5-1.6-7.4 3.5 1.7 5.6 3.8 8 5.9-1.5-6.3-3-12.6-4.8-19.2-1-3.7-4-8.1-7-12.5-2.1-2.7-4.3-5.2-7-6.9-3.3-2.1-6.7-3.8-9.7-5.5"
              fill="#fff"
            />
          </svg>
          Rescile Vault
        </a>
      </h1>
      <div className="top-nav">
        {clientname ? (
          <span className="client-badge" title="Authenticated client">
            {clientname}
          </span>
        ) : null}
        {onLogout ? (
          <button type="button" className="link-button" onClick={onLogout}>
            Sign out
          </button>
        ) : null}
        <div className="nav-item">
          <button
            onClick={onToggleTheme}
            className="theme-toggle"
            aria-label="Toggle theme"
          >
            {theme === 'dark' ? (
              <svg xmlns="http://www.w3.org/2000/svg" height="24px" viewBox="0 -960 960 960" width="24px" fill="currentColor">
                <path d="M480-120q-150 0-255-105T120-480q0-150 105-255t255-105q14 0 27.5 1t26.5 3q-41 29-65.5 75.5T444-660q0 90 63 153t153 63q55 0 101-24.5t75-65.5q2 13 3 26.5t1 27.5q0 150-105 255T480-120Zm0-80q88 0 158-48.5T740-375q-20 5-40 8t-40 3q-123 0-209.5-86.5T364-660q0-20 3-40t8-40q-78 32-126.5 102T200-480q0 116 82 198t198 82Zm-10-270Z" />
              </svg>
            ) : (
              <svg xmlns="http://www.w3.org/2000/svg" height="24px" viewBox="0 -960 960 960" width="24px" fill="currentColor">
                <path d="M480-360q50 0 85-35t35-85q0-50-35-85t-85-35q-50 0-85 35t-35 85q0 50 35 85t85 35Zm0 80q-83 0-141.5-58.5T280-480q0-83 58.5-141.5T480-680q83 0 141.5 58.5T680-480q0 83-58.5 141.5T480-280ZM200-440H40v-80h160v80Zm720 0H760v-80h160v80ZM440-760v-160h80v160h-80Zm0 720v-160h80v160h-80ZM256-650l-101-97 57-59 96 100-52 56Zm492 496-97-101 53-55 101 97-57 59Zm-98-550 97-101 59 57-100 96-56-52ZM154-212l101-97 55 53-97 101-59-57Zm326-268Z" />
              </svg>
            )}
          </button>
        </div>
      </div>
    </div>
  )
}

function LoginCard({
  config,
  onAuthenticated,
}: {
  config: Config
  onAuthenticated: () => void
}) {
  const [url, setUrl] = useState(config.url || '')
  const [clientname, setClientname] = useState(config.clientname || '')
  const [password, setPassword] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault()
    setError('')
    setBusy(true)
    const { ok, data } = await api<{ ok?: boolean; error?: string }>('/api/login', {
      method: 'POST',
      body: JSON.stringify({ url, clientname, password }),
    })
    setBusy(false)
    if (ok) {
      setPassword('')
      onAuthenticated()
    } else {
      setError(data.error || 'Login failed')
    }
  }

  return (
    <div className="card">
      <h2>Connect to Vault</h2>
      <p className="muted">
        Provide the credentials used to authenticate this client. Values from environment
        variables are pre-filled.
      </p>
      <form onSubmit={handleSubmit} className="form-grid">
        <label htmlFor="login-url">Vault URL</label>
        <input
          id="login-url"
          value={url}
          onChange={e => setUrl(e.target.value)}
          autoComplete="off"
          spellCheck={false}
          required
        />
        <label htmlFor="login-client">Client name</label>
        <input
          id="login-client"
          value={clientname}
          onChange={e => setClientname(e.target.value)}
          autoComplete="off"
          spellCheck={false}
          required
        />
        <label htmlFor="login-pass">Master password</label>
        <input
          id="login-pass"
          type="password"
          value={password}
          onChange={e => setPassword(e.target.value)}
          autoComplete="current-password"
          required
        />
        {error ? <div className="msg-err" role="alert">{error}</div> : null}
        <div className="form-actions">
          <button className="button" type="submit" disabled={busy}>
            {busy ? 'Connecting…' : 'Connect'}
          </button>
        </div>
      </form>
    </div>
  )
}

function SecretRowItem({
  row,
  collection,
  onChange,
  onRemove,
}: {
  row: SecretRow
  collection: string
  onChange: (patch: Partial<SecretRow>) => void
  onRemove: () => void
}) {
  const fileInputRef = useRef<HTMLInputElement>(null)

  async function call(
    path: string,
    body: Record<string, unknown>,
  ): Promise<{ ok: boolean; data: { value?: string; generated?: boolean; error?: string; file_base64?: string; is_binary?: boolean } }> {
    return api(path, { method: 'POST', body: JSON.stringify(body) })
  }

  async function handleGet() {
    onChange({ busy: true, message: null, value: '', revealed: false })
    const { ok, data } = await call('/api/secret/get', { collection, name: row.name })
    if (ok) {
      onChange({
        busy: false,
        value: data.value || '',
        revealed: true,
        message: {
          kind: 'ok',
          text: data.generated
            ? 'Did not exist — generated and stored a new value.'
            : data.is_binary ? 'Fetched binary secret.' : 'Fetched.',
        },
      })
    } else {
      onChange({ busy: false, message: { kind: 'err', text: data.error || 'Failed to fetch.' } })
    }
  }

  async function handleSave() {
    onChange({ busy: true, message: null })
    const { ok, data } = await call('/api/secret/put', {
      collection,
      name: row.name,
      value: row.value,
    })
    if (ok) {
      onChange({
        busy: false,
        value: data.value || row.value,
        revealed: data.generated ? true : row.revealed,
        message: {
          kind: 'ok',
          text: data.generated ? 'Value was empty — generated and saved.' : 'Saved.',
        },
      })
    } else {
      onChange({ busy: false, message: { kind: 'err', text: data.error || 'Failed to save.' } })
    }
  }

  async function handleGenerate() {
    onChange({ busy: true, message: null })
    const { ok, data } = await call('/api/secret/put', { collection, name: row.name })
    if (ok) {
      onChange({
        busy: false,
        value: data.value || '',
        revealed: true,
        message: { kind: 'ok', text: 'Generated and stored.' },
      })
    } else {
      onChange({ busy: false, message: { kind: 'err', text: data.error || 'Failed to generate.' } })
    }
  }

  async function handleDownload() {
    onChange({ busy: true, message: null })
    const { ok, data } = await call('/api/secret/get', { collection, name: row.name })
    if (ok) {
      if (data.file_base64) {
        const binStr = atob(data.file_base64)
        const bytes = new Uint8Array(binStr.length)
        for (let i = 0; i < binStr.length; i++) {
          bytes[i] = binStr.charCodeAt(i)
        }
        const blob = new Blob([bytes], { type: 'application/octet-stream' })
        const url = URL.createObjectURL(blob)
        const a = document.createElement('a')
        a.href = url
        a.download = row.name
        document.body.appendChild(a)
        a.click()
        document.body.removeChild(a)
        URL.revokeObjectURL(url)
      }
      onChange({
        busy: false,
        message: { kind: 'ok', text: 'Downloaded.' }
      })
    } else {
      onChange({ busy: false, message: { kind: 'err', text: data.error || 'Failed to download.' } })
    }
  }

  async function handleUpload(e: ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0]
    if (!file) return
    e.target.value = ''
    if (file.size > 1024 * 1024) {
      onChange({ busy: false, message: { kind: 'err', text: 'File exceeds the maximum allowed size of 1 Megabyte.' } })
      return
    }
    onChange({ busy: true, message: null })

    const reader = new FileReader()
    reader.onload = async () => {
      const bytes = new Uint8Array(reader.result as ArrayBuffer)
      let binary = ''
      for (let i = 0; i < bytes.byteLength; i++) {
        binary += String.fromCharCode(bytes[i])
      }
      const file_base64 = btoa(binary)

      const { ok, data } = await call('/api/secret/put', {
        collection,
        name: row.name,
        file_base64,
      })
      if (ok) {
        onChange({
          busy: false,
          value: data.value || '',
          revealed: false,
          message: { kind: 'ok', text: 'Uploaded.' },
        })
      } else {
        onChange({ busy: false, message: { kind: 'err', text: data.error || 'Failed to upload.' } })
      }
    }
    reader.onerror = () => {
      onChange({ busy: false, message: { kind: 'err', text: 'Failed to read file.' } })
    }
    reader.readAsArrayBuffer(file)
  }

  async function handleDelete() {
    if (!window.confirm(`Delete secret "${row.name}" from "${collection}"?`)) return
    onChange({ busy: true, message: null })
    const { ok, data } = await call('/api/secret/delete', { collection, name: row.name })
    if (ok) {
      onChange({ busy: false, value: '', revealed: false, message: { kind: 'ok', text: 'Deleted.' } })
    } else {
      onChange({ busy: false, message: { kind: 'err', text: data.error || 'Failed to delete.' } })
    }
  }

  async function handleCopy() {
    if (!row.value) return
    try {
      await navigator.clipboard.writeText(row.value)
      onChange({ message: { kind: 'ok', text: 'Copied to clipboard.' } })
    } catch {
      onChange({ message: { kind: 'err', text: 'Clipboard unavailable.' } })
    }
  }

  return (
    <div className="secret-row">
      <div className="secret-row-head">
        <span className="secret-row-name">{row.name}</span>
        <button
          type="button"
          className="icon-button"
          onClick={onRemove}
          aria-label="Close row"
          title="Remove from view"
        >
          ×
        </button>
      </div>

      <div className="value-row">
        <input
          type={row.revealed ? 'text' : 'password'}
          value={row.value}
          onChange={e => onChange({ value: e.target.value })}
          placeholder="(empty when saving = generate)"
          autoComplete="off"
          spellCheck={false}
        />
        <button
          type="button"
          className="secondary"
          onClick={() => onChange({ revealed: !row.revealed })}
          disabled={!row.value}
        >
          {row.revealed ? 'Hide' : 'Show'}
        </button>
        <button
          type="button"
          className="secondary"
          onClick={handleCopy}
          disabled={!row.value}
        >
          Copy
        </button>
      </div>

      <div className="action-row">
        <button type="button" className="button" onClick={handleGet} disabled={row.busy}>
          Get
        </button>
        <button
          type="button"
          className="button"
          onClick={handleSave}
          disabled={row.busy || !row.value}
        >
          Save
        </button>
        <button type="button" className="button" onClick={handleGenerate} disabled={row.busy}>
          Generate
        </button>
        <button type="button" className="button" onClick={handleDownload} disabled={row.busy}>
          Download
        </button>
        <button type="button" className="button" onClick={() => fileInputRef.current?.click()} disabled={row.busy}>
          Upload
        </button>
        <input ref={fileInputRef} type="file" onChange={handleUpload} style={{ display: 'none' }} />
        <button
          type="button"
          className="button danger"
          onClick={handleDelete}
          disabled={row.busy}
        >
          Delete
        </button>
      </div>

      {row.message ? (
        <div className={row.message.kind === 'ok' ? 'msg-ok' : 'msg-err'}>{row.message.text}</div>
      ) : null}
    </div>
  )
}

function VaultBrowser({ onLogout }: { onLogout: () => void }) {
  const initialParams = useMemo(() => new URLSearchParams(window.location.search), [])

  const [state, setState] = useState<State | null>(null)
  const [stateError, setStateError] = useState('')
  const [loadingState, setLoadingState] = useState(true)

  const [collectionInput, setCollectionInput] = useState(initialParams.get('collection') || '')
  const [collectionMessage, setCollectionMessage] = useState<Message | null>(null)
  const [creatingCollection, setCreatingCollection] = useState(false)

  const [secretRows, setSecretRows] = useState<SecretRow[]>([])
  const [newSecretName, setNewSecretName] = useState('')

  const [inviteClient, setInviteClient] = useState('')
  const [inviteValidity, setInviteValidity] = useState('')
  const [inviteMessage, setInviteMessage] = useState<Message | null>(null)
  const [busyInvite, setBusyInvite] = useState(false)
  const [issuedInvites, setIssuedInvites] = useState<IssuedInvite[]>([])

  const initialSecretConsumed = useRef(false)

  const refresh = useCallback(async () => {
    setLoadingState(true)
    const { ok, data, status } = await api<State>('/api/state')
    setLoadingState(false)
    if (ok) {
      setState(data)
      setStateError('')
    } else {
      setStateError(data.error || `Failed to load state (${status})`)
      if (status === 401) onLogout()
    }
  }, [onLogout])

  useEffect(() => {
    refresh()
  }, [refresh])

  const trimmedCollection = collectionInput.trim()
  const activeCollection = useMemo(
    () => state?.collections.find(c => c.name === trimmedCollection) || null,
    [state, trimmedCollection],
  )
  const collectionExistsAndAccessible = !!activeCollection?.has_access
  const collectionPendingInvite = !!activeCollection?.pending_invite

  // Keep URL in sync with the active collection + first open secret row.
  useEffect(() => {
    const params = new URLSearchParams()
    if (trimmedCollection) params.set('collection', trimmedCollection)
    if (secretRows[0]?.name) params.set('secret', secretRows[0].name)
    const qs = params.toString()
    const next = qs ? `${window.location.pathname}?${qs}` : window.location.pathname
    window.history.replaceState(null, '', next)
  }, [trimmedCollection, secretRows])

  // Open the secret row referenced by URL exactly once after state loads.
  useEffect(() => {
    if (initialSecretConsumed.current) return
    if (!state) return
    const wantedSecret = initialParams.get('secret')
    if (!wantedSecret) {
      initialSecretConsumed.current = true
      return
    }
    if (!trimmedCollection) return
    if (!state.collections.some(c => c.name === trimmedCollection && c.has_access)) return
    initialSecretConsumed.current = true
    addSecretRow(wantedSecret, true)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, initialParams, trimmedCollection])

  function addSecretRow(name: string, autoFetch = false) {
    const trimmed = name.trim()
    if (!trimmed) return
    const existing = secretRows.find(r => r.name === trimmed)
    if (existing) return
    const row: SecretRow = {
      id: newId(),
      name: trimmed,
      value: '',
      revealed: false,
      busy: false,
      message: null,
    }
    setSecretRows(rows => [row, ...rows])
    if (autoFetch) {
      // Fetch via the API directly so we can update this freshly-created row.
      void (async () => {
        const { ok, data } = await api<{ value?: string; generated?: boolean; error?: string; is_binary?: boolean }>(
          '/api/secret/get',
          {
            method: 'POST',
            body: JSON.stringify({ collection: trimmedCollection, name: trimmed }),
          },
        )
        setSecretRows(rs =>
          rs.map(r =>
            r.id === row.id
              ? {
                  ...r,
                  value: ok ? data.value || '' : '',
                  revealed: ok,
                  message: ok
                    ? {
                        kind: 'ok',
                        text: data.generated
                          ? 'Did not exist — generated and stored a new value.'
                          : data.is_binary ? 'Fetched binary secret.' : 'Fetched.',
                      }
                    : { kind: 'err', text: data.error || 'Failed to fetch.' },
                }
              : r,
          ),
        )
      })()
    }
  }

  function patchSecretRow(id: string, patch: Partial<SecretRow>) {
    setSecretRows(rs => rs.map(r => (r.id === id ? { ...r, ...patch } : r)))
  }

  function removeSecretRow(id: string) {
    setSecretRows(rs => rs.filter(r => r.id !== id))
  }

  async function createActiveCollection() {
    if (!trimmedCollection) return
    setCreatingCollection(true)
    setCollectionMessage(null)
    const { ok, data } = await api<{ ok?: boolean; error?: string }>('/api/collection', {
      method: 'POST',
      body: JSON.stringify({ name: trimmedCollection }),
    })
    setCreatingCollection(false)
    if (ok) {
      setCollectionMessage({ kind: 'ok', text: `Collection "${trimmedCollection}" created.` })
      refresh()
    } else {
      setCollectionMessage({ kind: 'err', text: data.error || 'Failed to create collection.' })
    }
  }

  function handleAddSecret(e: FormEvent) {
    e.preventDefault()
    if (!newSecretName.trim()) return
    addSecretRow(newSecretName)
    setNewSecretName('')
  }

  async function handleInvite(e: FormEvent) {
    e.preventDefault()
    if (!inviteClient.trim() || !trimmedCollection) return
    setBusyInvite(true)
    setInviteMessage(null)
    const payload = {
      collection: trimmedCollection,
      create_if_missing: false,
      secrets: [],
      invites: [
        {
          client: inviteClient.trim(),
          validity: inviteValidity.trim() || undefined,
        },
      ],
    }
    type BatchInviteResult = { client: string; status: string; token?: string; error?: string }
    type BatchResult = { invites: BatchInviteResult[]; error?: string }
    const { ok, data } = await api<BatchResult>('/api/batch', {
      method: 'POST',
      body: JSON.stringify(payload),
    })
    setBusyInvite(false)
    if (!ok) {
      setInviteMessage({ kind: 'err', text: data.error || 'Invite failed.' })
      return
    }
    const result = data.invites[0]
    if (!result) {
      setInviteMessage({ kind: 'err', text: 'Server returned no invite result.' })
      return
    }
    if (result.status === 'invited' && result.token) {
      setIssuedInvites(prev => [
        {
          id: newId(),
          client: result.client,
          token: result.token!,
          validity: inviteValidity.trim() || undefined,
        },
        ...prev,
      ])
      setInviteMessage({ kind: 'ok', text: `Invited "${result.client}".` })
      setInviteClient('')
      setInviteValidity('')
      refresh()
    } else {
      setInviteMessage({ kind: 'err', text: result.error || 'Invite failed.' })
    }
  }

  async function copyInviteToken(token: string) {
    try {
      await navigator.clipboard.writeText(token)
    } catch {
      /* ignore */
    }
  }

  function dismissInvite(id: string) {
    setIssuedInvites(prev => prev.filter(i => i.id !== id))
  }

  const collectionsList = state?.collections ?? []
  const showCreateButton =
    !!trimmedCollection && !activeCollection // not in our accessible list

  return (
    <>
      <div className="card">
        <h2>Collection</h2>
        <p className="muted">
          Pick an existing collection or type a new name to create one. All operations below
          apply to the selected collection.
        </p>

        <form
          onSubmit={e => {
            e.preventDefault()
            if (showCreateButton) void createActiveCollection()
          }}
          className="collection-picker"
        >
          <input
            list="collection-list"
            value={collectionInput}
            onChange={e => setCollectionInput(e.target.value)}
            placeholder="e.g. db_access"
            autoComplete="off"
            spellCheck={false}
          />
          <datalist id="collection-list">
            {collectionsList.map(c => (
              <option key={c.name} value={c.name} />
            ))}
          </datalist>
          {showCreateButton ? (
            <button
              type="submit"
              className="button"
              disabled={creatingCollection}
            >
              {creatingCollection ? 'Creating…' : `Create "${trimmedCollection}"`}
            </button>
          ) : null}
        </form>

        {loadingState && !state ? <p className="muted">Loading…</p> : null}
        {stateError ? <div className="msg-err">{stateError}</div> : null}

        {collectionsList.length > 0 ? (
          <div className="collection-chips">
            {collectionsList.map(c => {
              const active = c.name === trimmedCollection
              return (
                <button
                  type="button"
                  key={c.name}
                  className={active ? 'collection-chip active' : 'collection-chip'}
                  onClick={() => setCollectionInput(c.name)}
                >
                  {c.name}
                  {c.pending_invite ? <span className="badge">pending</span> : null}
                </button>
              )
            })}
          </div>
        ) : null}

        {!loadingState && trimmedCollection && !activeCollection ? (
          <p className="muted">
            No accessible collection named <code>{trimmedCollection}</code>. Click create above
            to make it.
          </p>
        ) : null}
        {collectionPendingInvite && !collectionExistsAndAccessible ? (
          <p className="muted">
            You have a pending invite for <code>{trimmedCollection}</code>. Fetch any secret in
            it to claim the invite and gain access.
          </p>
        ) : null}
        {collectionMessage ? (
          <div className={collectionMessage.kind === 'ok' ? 'msg-ok' : 'msg-err'}>
            {collectionMessage.text}
          </div>
        ) : null}
      </div>

      {collectionExistsAndAccessible || collectionPendingInvite ? (
        <div className="card">
          <h2>Secrets</h2>
          <p className="muted">
            Open a secret by name to view, rotate, or replace its value. Open multiple to work
            with several at once.
          </p>

          <form onSubmit={handleAddSecret} className="inline-form">
            <input
              value={newSecretName}
              onChange={e => setNewSecretName(e.target.value)}
              placeholder="secret name (e.g. DB_PASS)"
              autoComplete="off"
              spellCheck={false}
            />
            <button type="submit" className="button" disabled={!newSecretName.trim()}>
              Open
            </button>
          </form>

          {secretRows.length === 0 ? (
            <p className="muted">No secrets open. Add one above to begin.</p>
          ) : (
            <div className="secret-list">
              {secretRows.map(row => (
                <SecretRowItem
                  key={row.id}
                  row={row}
                  collection={trimmedCollection}
                  onChange={patch => patchSecretRow(row.id, patch)}
                  onRemove={() => removeSecretRow(row.id)}
                />
              ))}
            </div>
          )}
        </div>
      ) : null}

      {collectionExistsAndAccessible ? (
        <div className="card">
          <h2>Clients</h2>
          <p className="muted">
            Members of this collection, and a form to invite a new client.
          </p>

          <div className="member-chips">
            {activeCollection!.members.length === 0 &&
            activeCollection!.pending_invitees.length === 0 ? (
              <span className="muted">No members.</span>
            ) : null}
            {activeCollection!.members.map(m => (
              <span
                key={`m-${m}`}
                className={
                  m === state?.clientname ? 'member-chip self' : 'member-chip'
                }
                title={m === state?.clientname ? 'You' : undefined}
              >
                {m}
              </span>
            ))}
            {activeCollection!.pending_invitees.map(p => (
              <span key={`p-${p}`} className="member-chip pending">
                {p}
                <span className="badge">pending</span>
              </span>
            ))}
          </div>

          <form onSubmit={handleInvite} className="invite-form">
            <input
              value={inviteClient}
              onChange={e => setInviteClient(e.target.value)}
              placeholder="client name (e.g. postgres)"
              autoComplete="off"
              spellCheck={false}
            />
            <input
              value={inviteValidity}
              onChange={e => setInviteValidity(e.target.value)}
              placeholder="validity (e.g. 7d, optional)"
              autoComplete="off"
              spellCheck={false}
            />
            <button
              type="submit"
              className="button"
              disabled={busyInvite || !inviteClient.trim()}
            >
              {busyInvite ? 'Inviting…' : 'Invite'}
            </button>
          </form>

          {inviteMessage ? (
            <div className={inviteMessage.kind === 'ok' ? 'msg-ok' : 'msg-err'}>
              {inviteMessage.text}
            </div>
          ) : null}

          {issuedInvites.length > 0 ? (
            <div className="batch-section">
              <h3>Issued invites</h3>
              <ul className="result-list">
                {issuedInvites.map(inv => (
                  <li key={inv.id} className="result-item status-invited">
                    <div className="result-head">
                      <span className="result-name">{inv.client}</span>
                      {inv.validity ? <span className="badge">{inv.validity}</span> : null}
                      <button
                        type="button"
                        className="icon-button"
                        onClick={() => dismissInvite(inv.id)}
                        aria-label="Dismiss"
                        title="Dismiss"
                      >
                        ×
                      </button>
                    </div>
                    <div className="muted token-label">
                      Invite token (share with the invited client):
                    </div>
                    <div className="value-row">
                      <input readOnly value={inv.token} />
                      <button
                        type="button"
                        className="secondary"
                        onClick={() => copyInviteToken(inv.token)}
                      >
                        Copy
                      </button>
                    </div>
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
        </div>
      ) : null}
    </>
  )
}

function App() {
  const { theme, toggle } = useTheme()
  const [config, setConfig] = useState<Config | null>(null)
  const [configError, setConfigError] = useState('')
  const [authenticated, setAuthenticated] = useState(false)

  const loadConfig = useCallback(async () => {
    const { ok, data } = await api<Config>('/api/config')
    if (ok) {
      setConfig(data)
      setAuthenticated(!!data.ready)
      setConfigError('')
    } else {
      setConfigError(data.error || 'Failed to load configuration.')
    }
  }, [])

  useEffect(() => {
    loadConfig()
  }, [loadConfig])

  useEffect(() => {
    const root = document.getElementById('root')
    if (!root) return
    if (!authenticated) root.classList.add('login-mode')
    else root.classList.remove('login-mode')
  }, [authenticated])

  const handleLogout = useCallback(async () => {
    await api('/api/logout', { method: 'POST' })
    setAuthenticated(false)
  }, [])

  if (!config) {
    return (
      <>
        <Header theme={theme} onToggleTheme={toggle} />
        <div className="content">
          <div className="card">
            {configError ? <div className="msg-err">{configError}</div> : <p>Loading…</p>}
          </div>
        </div>
      </>
    )
  }

  return (
    <>
      <Header
        theme={theme}
        onToggleTheme={toggle}
        clientname={authenticated ? config.clientname : undefined}
        onLogout={authenticated ? handleLogout : undefined}
      />
      <div className="content">
        {authenticated ? (
          <VaultBrowser onLogout={() => setAuthenticated(false)} />
        ) : (
          <LoginCard
            config={config}
            onAuthenticated={() => {
              setAuthenticated(true)
              void loadConfig()
            }}
          />
        )}
      </div>
    </>
  )
}

export default App
