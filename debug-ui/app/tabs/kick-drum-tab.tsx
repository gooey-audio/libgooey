import React from "react";

interface KickConfig {
  frequency: number;
  punch: number;
  sub: number;
  click: number;
  decay: number;
  pitchDrop: number;
  volume: number;
}

interface KickDrumTabProps {
  isLoaded: boolean;
  isPlaying: boolean;
  kickPreset: string;
  kickConfig: KickConfig;
  triggerKickDrum: () => void;
  releaseKickDrum: () => void;
  handleKickConfigChange: (param: keyof KickConfig, value: number) => void;
  handleKickPresetChange: (preset: string) => void;
}

export default function KickDrumTab({
  isLoaded,
  isPlaying,
  kickPreset,
  kickConfig,
  triggerKickDrum,
  releaseKickDrum,
  handleKickConfigChange,
  handleKickPresetChange,
}: KickDrumTabProps) {
  return (
    <div>
      <h3 className="font-semibold mb-4 text-center text-lg">🥁 Kick Drum</h3>

      {/* Kick Drum Trigger Button */}
      <button
        onClick={triggerKickDrum}
        disabled={!isLoaded || !isPlaying}
        className="w-full px-4 py-3 mb-4 bg-gradient-to-r from-red-600 to-red-700 text-white rounded-lg hover:from-red-700 hover:to-red-800 disabled:bg-gray-400 disabled:cursor-not-allowed font-semibold text-lg shadow-lg"
      >
        🥁 TRIGGER KICK
      </button>

      {/* Preset Selection */}
      <div className="mb-4">
        <label className="block text-sm font-medium mb-2">Preset</label>
        <select
          value={kickPreset}
          onChange={(e) => handleKickPresetChange(e.target.value)}
          disabled={!isLoaded}
          className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded disabled:cursor-not-allowed"
        >
          <option value="default">Default</option>
          <option value="punchy">Punchy</option>
          <option value="deep">Deep</option>
          <option value="tight">Tight</option>
        </select>
      </div>

      {/* Kick Drum Controls */}
      <div className="space-y-3">
        {/* Frequency */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Frequency</label>
          <input
            type="range"
            min="20"
            max="200"
            step="1"
            value={kickConfig.frequency}
            onChange={(e) =>
              handleKickConfigChange("frequency", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {kickConfig.frequency.toFixed(0)}Hz
          </span>
        </div>

        {/* Volume */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Volume</label>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={kickConfig.volume}
            onChange={(e) =>
              handleKickConfigChange("volume", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {kickConfig.volume.toFixed(2)}
          </span>
        </div>

        {/* Decay Time */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Decay</label>
          <input
            type="range"
            min="0.1"
            max="3"
            step="0.01"
            value={kickConfig.decay}
            onChange={(e) =>
              handleKickConfigChange("decay", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {kickConfig.decay.toFixed(2)}s
          </span>
        </div>

        {/* Punch Amount */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Punch</label>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={kickConfig.punch}
            onChange={(e) =>
              handleKickConfigChange("punch", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {kickConfig.punch.toFixed(2)}
          </span>
        </div>

        {/* Sub Amount */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Sub Bass</label>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={kickConfig.sub}
            onChange={(e) =>
              handleKickConfigChange("sub", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {kickConfig.sub.toFixed(2)}
          </span>
        </div>

        {/* Click Amount */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Click</label>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={kickConfig.click}
            onChange={(e) =>
              handleKickConfigChange("click", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {kickConfig.click.toFixed(2)}
          </span>
        </div>

        {/* Pitch Drop */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Pitch Drop</label>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={kickConfig.pitchDrop}
            onChange={(e) =>
              handleKickConfigChange("pitchDrop", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {kickConfig.pitchDrop.toFixed(2)}
          </span>
        </div>
      </div>

      {/* Release Button */}
      <button
        onClick={releaseKickDrum}
        disabled={!isLoaded || !isPlaying}
        className="w-full mt-4 px-4 py-2 bg-gray-600 text-white rounded hover:bg-gray-500 disabled:bg-gray-700 disabled:cursor-not-allowed"
      >
        Release Kick
      </button>
    </div>
  );
}