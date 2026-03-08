import { useState } from 'react'
import type { Comment } from '../types/comments'

interface CommentSectionProps {
  comments: Comment[]
  onCommentsChange: (comments: Comment[]) => void
  ko: boolean
}

export default function CommentSection({ comments, onCommentsChange, ko }: CommentSectionProps) {
  const [commentInput, setCommentInput] = useState('')
  const [replyingTo, setReplyingTo] = useState<string | null>(null)
  const [replyInput, setReplyInput] = useState('')

  return (
    <>
      {/* Comments header */}
      <div className="flex items-center justify-between px-1">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">
          {ko ? `댓글 ${comments.length}개` : `${comments.length} comments`}
        </span>
        <select className="text-[10px] bg-transparent text-[var(--color-text-secondary)] outline-none cursor-pointer">
          <option>{ko ? '최신순' : 'Newest'}</option>
          <option>{ko ? '인기순' : 'Popular'}</option>
        </select>
      </div>

      {/* Comments List */}
      <div className="space-y-2">
        {comments.map(comment => (
          <div key={comment.id} className="bg-[var(--color-bg-sidebar)] rounded-xl border border-[var(--color-border)] overflow-hidden">
            {/* Main comment */}
            <div className="p-3">
              <div className="flex items-start gap-2.5">
                <div className="w-7 h-7 rounded-full bg-[var(--color-border)] flex items-center justify-center text-[10px] font-bold flex-shrink-0">
                  {comment.avatar}
                </div>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-[12px] font-semibold">{comment.author}</span>
                    <span className="text-[9px] text-[var(--color-text-secondary)]">{comment.time}</span>
                  </div>
                  <p className="text-[12px] mt-1 leading-relaxed">{comment.text}</p>
                  <div className="flex items-center gap-3 mt-2">
                    <button
                      onClick={() => {
                        onCommentsChange(comments.map(c => c.id === comment.id ? { ...c, liked: !c.liked, likes: c.liked ? c.likes - 1 : c.likes + 1 } : c))
                      }}
                      className={`flex items-center gap-1 text-[10px] cursor-pointer transition-colors ${comment.liked ? 'text-[#3b82f6]' : 'text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'}`}
                    >
                      <svg width="12" height="12" viewBox="0 0 24 24" fill={comment.liked ? '#3b82f6' : 'none'} stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"/>
                      </svg>
                      {comment.likes > 0 && comment.likes}
                    </button>
                    <button
                      onClick={() => setReplyingTo(replyingTo === comment.id ? null : comment.id)}
                      className="text-[10px] text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer"
                    >
                      {ko ? '답글' : 'Reply'}
                    </button>
                  </div>
                </div>
              </div>
            </div>

            {/* Replies */}
            {comment.replies.length > 0 && (
              <div className="border-t border-[var(--color-border)] bg-[var(--color-bg-main)]">
                {comment.replies.map(reply => (
                  <div key={reply.id} className="px-3 py-2.5 ml-6 border-b border-[var(--color-border)] last:border-b-0">
                    <div className="flex items-start gap-2">
                      <div className="w-5 h-5 rounded-full bg-[var(--color-border)] flex items-center justify-center text-[8px] font-bold flex-shrink-0">
                        {reply.avatar}
                      </div>
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="text-[11px] font-semibold">{reply.author}</span>
                          <span className="text-[9px] text-[var(--color-text-secondary)]">{reply.time}</span>
                        </div>
                        <p className="text-[11px] mt-0.5 leading-relaxed">{reply.text}</p>
                        <button
                          onClick={() => {
                            onCommentsChange(comments.map(c => c.id === comment.id
                              ? { ...c, replies: c.replies.map(r => r.id === reply.id ? { ...r, liked: !r.liked, likes: r.liked ? r.likes - 1 : r.likes + 1 } : r) }
                              : c))
                          }}
                          className={`flex items-center gap-1 text-[9px] mt-1 cursor-pointer transition-colors ${reply.liked ? 'text-[#3b82f6]' : 'text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'}`}
                        >
                          <svg width="10" height="10" viewBox="0 0 24 24" fill={reply.liked ? '#3b82f6' : 'none'} stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                            <path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"/>
                          </svg>
                          {reply.likes > 0 && reply.likes}
                        </button>
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            )}

            {/* Reply Input */}
            {replyingTo === comment.id && (
              <div className="border-t border-[var(--color-border)] p-3 bg-[var(--color-bg-main)]">
                <div className="flex items-start gap-2 ml-6">
                  <div className="w-5 h-5 rounded-full bg-[var(--color-accent)] flex items-center justify-center text-[8px] font-bold text-[var(--color-accent-text)] flex-shrink-0 mt-0.5">
                    Me
                  </div>
                  <div className="flex-1">
                    {(() => {
                      const submitReply = () => {
                        if (!replyInput.trim()) return
                        const newReply: Comment = {
                          id: `reply-${Date.now()}`, author: 'me', avatar: 'Me', text: replyInput.trim(),
                          time: ko ? '방금' : 'Just now', likes: 0, liked: false, replies: [],
                        }
                        onCommentsChange(comments.map(c => c.id === comment.id ? { ...c, replies: [...c.replies, newReply] } : c))
                        setReplyInput('')
                        setReplyingTo(null)
                      }
                      return (<>
                    <input
                      type="text"
                      value={replyInput}
                      onChange={e => setReplyInput(e.target.value)}
                      placeholder={ko ? '답글을 입력하세요...' : 'Write a reply...'}
                      onKeyDown={e => { if (e.key === 'Enter') submitReply() }}
                      className="w-full bg-[var(--color-bg-sidebar)] rounded-lg px-2.5 py-1.5 text-[11px] outline-none border border-[var(--color-border)]"
                      autoFocus
                    />
                    <div className="flex items-center gap-2 mt-1.5">
                      <button
                        onClick={submitReply}
                        disabled={!replyInput.trim()}
                        className="bg-[#3b82f6] text-white text-[10px] font-medium px-3 py-1 rounded-lg hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-40"
                      >
                        {ko ? '등록' : 'Post'}
                      </button>
                      <button
                        onClick={() => { setReplyingTo(null); setReplyInput('') }}
                        className="text-[10px] text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer"
                      >
                        {ko ? '취소' : 'Cancel'}
                      </button>
                    </div>
                      </>)
                    })()}
                  </div>
                </div>
              </div>
            )}
          </div>
        ))}
      </div>

      {/* Write new comment */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
        <div className="flex items-start gap-2.5">
          <div className="w-7 h-7 rounded-full bg-[var(--color-accent)] flex items-center justify-center text-[10px] font-bold text-[var(--color-accent-text)] flex-shrink-0 mt-0.5">
            Me
          </div>
          <div className="flex-1">
            <textarea
              value={commentInput}
              onChange={e => setCommentInput(e.target.value)}
              placeholder={ko ? '질문이나 의견을 남겨보세요...' : 'Ask a question or leave a comment...'}
              rows={2}
              className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-[12px] outline-none border border-[var(--color-border)] resize-none"
            />
            <div className="flex justify-end mt-1.5">
              <button
                onClick={() => {
                  if (!commentInput.trim()) return
                  const newComment: Comment = {
                    id: `new-${Date.now()}`, author: 'me', avatar: 'Me', text: commentInput.trim(),
                    time: ko ? '방금' : 'Just now', likes: 0, liked: false, replies: [],
                  }
                  onCommentsChange([newComment, ...comments])
                  setCommentInput('')
                }}
                disabled={!commentInput.trim()}
                className="bg-[#3b82f6] text-white text-[11px] font-medium px-4 py-1.5 rounded-lg hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-40"
              >
                {ko ? '등록' : 'Post'}
              </button>
            </div>
          </div>
        </div>
      </div>
    </>
  )
}
