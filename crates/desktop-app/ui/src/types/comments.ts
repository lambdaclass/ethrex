export interface Comment {
  id: string
  author: string
  avatar: string
  text: string
  time: string
  likes: number
  liked: boolean
  replies: Comment[]
}
