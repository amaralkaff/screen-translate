import { useState, useEffect } from 'react'
import { motion } from 'framer-motion'
import TextType from './components/TextType'
import GradientText from './components/GradientText'
import GlassIcons from './components/GlassIcons'
import Silk from './components/Silk'
import './App.css'

function App() {
  const [latestRelease, setLatestRelease] = useState(null)

  useEffect(() => {
    fetch('https://api.github.com/repos/amaralkaff/screen-translate/releases/latest')
      .then(res => res.json())
      .then(data => setLatestRelease(data))
      .catch(err => console.error('Failed to fetch release:', err))
  }, [])

  // Dynamic URLs that auto-update with latest release
  const downloadUrl = latestRelease?.assets?.find(asset =>
    asset.name.includes('Full') && asset.name.endsWith('.exe')
  )?.browser_download_url || `https://github.com/amaralkaff/screen-translate/releases/latest`

  const macOSUrl = latestRelease?.assets?.find(asset =>
    asset.name.endsWith('.dmg')
  )?.browser_download_url || `https://github.com/amaralkaff/screen-translate/releases/latest`

  const downloadItems = [
    downloadUrl && {
      icon: (
        <svg className="w-6 h-6" fill="currentColor" viewBox="0 0 24 24">
          <path d="M0 3.449L9.75 2.1v9.451H0m10.949-9.602L24 0v11.4H10.949M0 12.6h9.75v9.451L0 20.699M10.949 12.6H24V24l-12.9-1.801"/>
        </svg>
      ),
      color: '#2b7cba',
      label: 'Windows',
      href: downloadUrl
    },
    macOSUrl && {
      icon: (
        <svg className="w-8 h-8" fill="currentColor" viewBox="0 0 24 24">
          <path d="M18.71 19.5c-.83 1.24-1.71 2.45-3.05 2.47-1.34.03-1.77-.79-3.29-.79-1.53 0-2 .77-3.27.82-1.31.05-2.3-1.32-3.14-2.53C4.25 17 2.94 12.45 4.7 9.39c.87-1.52 2.43-2.48 4.12-2.51 1.28-.02 2.5.87 3.29.87.78 0 2.26-1.07 3.81-.91.65.03 2.47.26 3.64 1.98-.09.06-2.17 1.28-2.15 3.81.03 3.02 2.65 4.03 2.68 4.04-.03.07-.42 1.44-1.38 2.83M13 3.5c.73-.83 1.94-1.46 2.94-1.5.13 1.17-.34 2.35-1.04 3.19-.69.85-1.83 1.51-2.95 1.42-.15-1.15.41-2.35 1.05-3.11z"/>
        </svg>
      ),
      color: '#5ba3d0',
      label: 'macOS',
      href: macOSUrl
    }
  ].filter(Boolean)

  return (
    <div className="h-screen w-full overflow-hidden bg-gradient-to-br from-[#1a1d1f] via-[#2b3d4f] to-[#1a1d1f] flex items-center justify-center p-4 md:p-8 relative">
      {/* Silk background */}
      <div className="absolute inset-0 opacity-40">
        <Silk
          speed={5}
          scale={1}
          color="#3f4447"
          noiseIntensity={1.5}
          rotation={0}
        />
      </div>

      {/* Animated background grid */}
      <div className="absolute inset-0 bg-[linear-gradient(to_right,#4f4f4f2e_1px,transparent_1px),linear-gradient(to_bottom,#4f4f4f2e_1px,transparent_1px)] bg-[size:4rem_4rem] [mask-image:radial-gradient(ellipse_60%_50%_at_50%_0%,#000_70%,transparent_110%)]" />

      <div className="max-w-7xl w-full grid md:grid-cols-2 gap-8 md:gap-16 items-center relative z-10">
        {/* Left: Hero Content */}
        <motion.div
          className="space-y-4 md:space-y-5"
          initial={{ opacity: 0, x: -50 }}
          animate={{ opacity: 1, x: 0 }}
          transition={{ duration: 0.8 }}
        >
          <div className="space-y-3">
            <motion.h1
              className="text-3xl md:text-5xl lg:text-6xl font-bold text-white leading-tight"
              initial={{ opacity: 0, y: 20 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.3 }}
            >
              Screen
              <GradientText
                colors={['#2b7cba', '#5ba3d0', '#2b7cba']}
                className="block text-3xl md:text-5xl lg:text-6xl font-bold"
                animationSpeed={3}
              >
                Translate
              </GradientText>
            </motion.h1>

            <div className="h-16 md:h-20">
              <TextType
                text="Select text anywhere. Get instant translation. All local."
                className="text-base md:text-xl text-slate-300 leading-relaxed font-light"
                speed={30}
                cursorClassName="bg-[#2b7cba]"
              />
            </div>
          </div>

          <motion.div
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: 0.6 }}
          >
            <div onClick={(e) => {
              const target = e.target.closest('.icon-btn');
              if (target) {
                const item = downloadItems[Array.from(target.parentNode.children).indexOf(target)];
                if (item?.href) window.location.href = item.href;
              }
            }}>
              <GlassIcons items={downloadItems} colorful={true} />
            </div>
          </motion.div>

          <motion.div
            className="flex items-center gap-6 text-sm text-slate-400 mt-8 pt-8"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ delay: 0.8 }}
          >
            <a
              href="https://github.com/amaralkaff/screen-translate"
              target="_blank"
              rel="noopener noreferrer"
              className="flex items-center gap-2 hover:text-[#2b7cba] transition-colors group"
            >
              <svg className="w-5 h-5 group-hover:rotate-12 transition-transform" fill="currentColor" viewBox="0 0 24 24">
                <path fillRule="evenodd" d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z" clipRule="evenodd" />
              </svg>
              <span className="font-medium">GitHub</span>
            </a>
            <span className="text-slate-700">•</span>
            <span className="font-mono bg-slate-800/50 px-3 py-1 rounded-md border border-slate-700">
              {latestRelease?.tag_name || 'v0.1.0'}
            </span>
          </motion.div>
        </motion.div>

        {/* Right: Demo Section */}
        <motion.div
          initial={{ opacity: 0, x: 50 }}
          animate={{ opacity: 1, x: 0 }}
          transition={{ duration: 0.8, delay: 0.2 }}
        >
          {/* Demo GIF */}
          <motion.div
            className="relative rounded-2xl overflow-hidden border border-white/20 shadow-2xl shadow-[#2b7cba]/20"
            whileHover={{ scale: 1.02 }}
            transition={{ duration: 0.3 }}
          >
            <img
              src="/screen-translate/demo.gif"
              alt="Translation Demo"
              className="w-full h-auto"
            />
            <div className="absolute inset-0 bg-gradient-to-t from-black/60 via-transparent to-transparent pointer-events-none" />
          </motion.div>
        </motion.div>
      </div>
    </div>
  )
}

export default App
