import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle, DialogTrigger } from '@/components/ui/dialog'
import { DialogClose } from '@radix-ui/react-dialog'
import { CodeIcon } from '@radix-ui/react-icons'
import React, { useState } from 'react'

interface RawEnvEditorProps {
  owner: string
  project: string
  environmentVars: Record<string, string>
  onUpdate: () => void
  children: React.ReactNode
}

export function RawEnvEditor({ owner, project, environmentVars, onUpdate, children }: RawEnvEditorProps) {
  const [open, setOpen] = useState(false)
  const [rawText, setRawText] = useState('')
  const [isLoading, setIsLoading] = useState(false)
  const [error, setError] = useState('')

  // Convert env vars to raw text format
  const envToRawText = (envs: Record<string, string>) => {
    return Object.entries(envs)
      .map(([key, value]) => `${key}=${value}`)
      .join('\n')
  }

  // Parse raw text to env vars
  const parseRawText = (text: string): Record<string, string> => {
    const envs: Record<string, string> = {}
    const lines = text.split('\n').filter(line => line.trim() && !line.trim().startsWith('#'))
    
    for (const line of lines) {
      const equalIndex = line.indexOf('=')
      if (equalIndex > 0) {
        const key = line.substring(0, equalIndex).trim()
        const value = line.substring(equalIndex + 1).trim()
        
        // Handle quoted values
        const cleanValue = value.replace(/^["']|["']$/g, '')
        envs[key] = cleanValue
      }
    }
    
    return envs
  }

  const handleOpen = () => {
    setRawText(envToRawText(environmentVars))
    setError('')
    setOpen(true)
  }

  const handleSave = async () => {
    setIsLoading(true)
    setError('')
    
    try {
      const parsedEnvs = parseRawText(rawText)
      
      // Use bulk API endpoint
      const response = await fetch(`${import.meta.env.VITE_API_URL}/project/${owner}/${project}/env/bulk`, {
        credentials: "include",
        headers: {
          "Content-Type": "application/json"
        },
        method: "POST",
        body: JSON.stringify({ envs: parsedEnvs })
      })

      if (!response.ok) {
        const errorData = await response.json()
        throw new Error(errorData.message || 'Failed to update environment variables')
      }

      setOpen(false)
      onUpdate()
    } catch (error) {
      console.error('Error updating environment variables:', error)
      setError(error instanceof Error ? error.message : 'Failed to update environment variables')
    } finally {
      setIsLoading(false)
    }
  }



  return (
    <>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogTrigger onClick={handleOpen}>
          {children}
        </DialogTrigger>
        <DialogContent className="text-white max-w-4xl max-h-[80vh] flex flex-col">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <CodeIcon className="w-5 h-5" />
              Raw Environment Editor
            </DialogTitle>
          </DialogHeader>
          
          <div className="flex-1 space-y-4 overflow-hidden">
            {error && (
              <div className="bg-red-900/20 border border-red-500 text-red-400 px-4 py-2 rounded-md text-sm">
                {error}
              </div>
            )}

            <div className="space-y-2 flex-1 flex flex-col">
              <label className="text-sm font-medium">
                Environment Variables (KEY=value format, one per line)
              </label>
              <textarea
                value={rawText}
                onChange={(e) => setRawText(e.target.value)}
                placeholder={`# Example:
DATABASE_URL=postgresql://user:pass@localhost/db
API_KEY=your-secret-key
DEBUG=true
PORT=3000`}
                className="flex-1 w-full rounded-md border border-slate-600 bg-slate-900 px-3 py-2 text-sm text-white placeholder:text-slate-400 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent font-mono resize-none min-h-[300px]"
                spellCheck={false}
              />
              <p className="text-xs text-slate-400">
                Lines starting with # are treated as comments and will be ignored.
                Quotes around values will be automatically removed.
              </p>
            </div>
          </div>

          <DialogFooter className="flex-shrink-0 flex gap-2 pt-4">
            <DialogClose asChild>
              <Button variant="outline" disabled={isLoading}>
                Cancel
              </Button>
            </DialogClose>
            <Button 
              onClick={handleSave} 
              disabled={isLoading}
              className="text-foreground"
            >
              {isLoading ? 'Updating...' : 'Update All Variables'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
