import { useCallback, useEffect, useRef, useState } from 'react'
import { ApiError } from './client'

interface AsyncState<T> {
  data: T | null
  loading: boolean
  error: ApiError | null
}

/**
 * 轻量读取 hook：页面级数据、手动刷新；不做缓存/全局状态。
 * load 必须稳定（useCallback/模块函数），依赖放 deps。
 */
export function useAsync<T>(load: () => Promise<T>, deps: unknown[] = []) {
  const [state, setState] = useState<AsyncState<T>>({ data: null, loading: true, error: null })
  const [tick, setTick] = useState(0)
  const mounted = useRef(true)

  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])

  useEffect(() => {
    let alive = true
    setState((prev) => ({ ...prev, loading: true, error: null }))
    load()
      .then((data) => {
        if (alive) setState({ data, loading: false, error: null })
      })
      .catch((error: unknown) => {
        if (!alive) return
        const apiError = error instanceof ApiError ? error : new ApiError(0, { detail: '加载失败，请重试' }, null, null)
        setState({ data: null, loading: false, error: apiError })
      })
    return () => {
      alive = false
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tick, ...deps])

  const reload = useCallback(() => setTick((value) => value + 1), [])
  return { ...state, reload }
}
