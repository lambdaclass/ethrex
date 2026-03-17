import { useState, useEffect } from 'react'
import { useLang } from '../App'
import { platformAPI, type Program } from '../api/platform'

export default function ProgramStoreView() {
  const { lang } = useLang()
  const [programs, setPrograms] = useState<Program[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [search, setSearch] = useState('')
  const [category, setCategory] = useState('')
  const [categories, setCategories] = useState<string[]>([])
  const [selected, setSelected] = useState<Program | null>(null)

  useEffect(() => {
    Promise.all([
      platformAPI.getPrograms().catch(() => []),
      platformAPI.getCategories().catch(() => []),
    ]).then(([progs, cats]) => {
      setPrograms(progs)
      setCategories(cats)
    }).catch(e => setError(e.message))
      .finally(() => setLoading(false))
  }, [])

  const filtered = programs.filter(p => {
    const matchSearch = !search ||
      p.name.toLowerCase().includes(search.toLowerCase()) ||
      p.program_id.toLowerCase().includes(search.toLowerCase())
    const matchCat = !category || p.category === category
    return matchSearch && matchCat
  })

  if (selected) {
    return (
      <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
        <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
          <button
            onClick={() => setSelected(null)}
            className="text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer"
          >
            ← {lang === 'ko' ? '뒤로' : 'Back'}
          </button>
        </div>
        <div className="flex-1 overflow-y-auto p-4 space-y-4">
          <div className="flex items-center gap-4">
            <div className="w-14 h-14 rounded-xl bg-[var(--color-accent)] flex items-center justify-center text-2xl font-bold text-[var(--color-accent-text)]">
              {selected.name.charAt(0).toUpperCase()}
            </div>
            <div>
              <h1 className="text-lg font-semibold">{selected.name}</h1>
              <div className="text-[12px] text-[var(--color-text-secondary)] font-mono">{selected.program_id}</div>
              <div className="flex items-center gap-2 mt-1">
                <span className="text-[10px] px-2 py-0.5 rounded bg-[var(--color-tag-bg)] text-[var(--color-tag-text)]">
                  {selected.category}
                </span>
                {selected.is_official && (
                  <span className="text-[10px] px-2 py-0.5 rounded bg-[var(--color-accent)] text-[var(--color-accent-text)]">
                    Official
                  </span>
                )}
                <span className="text-[10px] text-[var(--color-text-secondary)]">
                  {selected.use_count} {lang === 'ko' ? '배포' : 'deployments'}
                </span>
              </div>
            </div>
          </div>

          <p className="text-[13px] text-[var(--color-text-secondary)]">
            {selected.description || (lang === 'ko' ? '설명 없음' : 'No description')}
          </p>

          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
            <h2 className="text-[13px] font-medium mb-2">
              {lang === 'ko' ? '이 프로그램으로 L2 만들기' : 'Create L2 with this program'}
            </h2>
            <p className="text-[11px] text-[var(--color-text-secondary)] mb-3">
              {lang === 'ko'
                ? 'My L2 메뉴에서 새 앱체인을 만들고 이 프로그램을 선택하세요.'
                : 'Go to My L2 menu and create a new appchain with this program.'}
            </p>
            <div className="text-[11px] text-[var(--color-text-secondary)] space-y-1 font-mono bg-[var(--color-bg-main)] p-3 rounded-lg border border-[var(--color-border)]">
              <div>program_id: {selected.program_id}</div>
              <div>category: {selected.category}</div>
              <div>status: {selected.status}</div>
            </div>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
        <h1 className="text-base font-semibold mb-2">
          {lang === 'ko' ? 'Program Store' : 'Program Store'}
        </h1>
        <div className="flex gap-2">
          <div className="relative flex-1">
            <input
              type="text"
              value={search}
              onChange={e => setSearch(e.target.value)}
              placeholder={lang === 'ko' ? '프로그램 검색...' : 'Search programs...'}
              className="w-full bg-[var(--color-bg-sidebar)] rounded-lg px-3 py-2 text-[13px] outline-none placeholder-[var(--color-text-secondary)] border border-[var(--color-border)] pl-8"
            />
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--color-text-secondary)]">
              <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
            </svg>
          </div>
          {categories.length > 0 && (
            <select
              value={category}
              onChange={e => setCategory(e.target.value)}
              className="bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] rounded-lg px-3 py-2 text-[13px] outline-none cursor-pointer"
            >
              <option value="">{lang === 'ko' ? '전체' : 'All'}</option>
              {categories.map(c => <option key={c} value={c}>{c}</option>)}
            </select>
          )}
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="flex items-center justify-center h-full">
            <div className="text-[var(--color-text-secondary)] text-[13px]">
              {lang === 'ko' ? '로딩 중...' : 'Loading...'}
            </div>
          </div>
        ) : error ? (
          <div className="flex flex-col items-center justify-center h-full text-center px-6">
            <p className="text-[var(--color-text-secondary)] text-[13px] mb-2">
              {lang === 'ko' ? 'Platform에 연결할 수 없습니다' : 'Cannot connect to Platform'}
            </p>
            <p className="text-[11px] text-[var(--color-text-secondary)]">{error}</p>
          </div>
        ) : filtered.length === 0 ? (
          <div className="flex items-center justify-center h-full text-[var(--color-text-secondary)] text-[13px]">
            {lang === 'ko' ? '프로그램이 없습니다' : 'No programs found'}
          </div>
        ) : (
          filtered.map(p => (
            <button
              key={p.id}
              onClick={() => setSelected(p)}
              className="w-full px-4 py-3 flex items-center gap-3 hover:bg-[var(--color-bg-sidebar)] transition-colors cursor-pointer border-b border-[var(--color-border)] text-left"
            >
              <div className="w-10 h-10 rounded-xl bg-[var(--color-accent)] flex items-center justify-center text-lg font-bold flex-shrink-0 text-[var(--color-accent-text)]">
                {p.name.charAt(0).toUpperCase()}
              </div>
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-1.5">
                  <span className="text-sm font-medium truncate">{p.name}</span>
                  {p.is_official && (
                    <span className="text-[9px] bg-[var(--color-accent)] px-1.5 py-0.5 rounded text-[var(--color-accent-text)] font-medium">Official</span>
                  )}
                </div>
                <div className="text-[11px] text-[var(--color-text-secondary)] truncate mt-0.5">
                  {p.description || p.program_id}
                </div>
                <div className="flex items-center gap-2 mt-1 text-[10px] text-[var(--color-text-secondary)]">
                  <span className="bg-[var(--color-tag-bg)] px-1.5 py-0.5 rounded text-[var(--color-tag-text)]">{p.category}</span>
                  <span>{p.use_count} {lang === 'ko' ? '배포' : 'uses'}</span>
                </div>
              </div>
            </button>
          ))
        )}
      </div>
    </div>
  )
}
