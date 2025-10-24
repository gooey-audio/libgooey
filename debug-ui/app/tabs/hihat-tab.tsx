import React from "react";

interface HiHatTabProps {
  isLoaded: boolean;
  isPlaying: boolean;
  hihatPreset: string;
  hihatConfig: {
    baseFrequency: number;
    resonance: number;
    brightness: number;
    decayTime: number;
    attackTime: number;
    volume: number;
    isOpen: boolean;
  };
  triggerHiHat: (preset?: string) => void;
  releaseHiHat: () => void;
  handleHihatConfigChange: (
    param: string,
    value: number | boolean
  ) => void;
  handleHihatPresetChange: (preset: string) => void;
}

export default function HiHatTab({
  isLoaded,
  isPlaying,
  hihatPreset,
  hihatConfig,
  triggerHiHat,
  releaseHiHat,
  handleHihatConfigChange,
  handleHihatPresetChange,
}: HiHatTabProps) {
  return (
    <div>
      <h3 className="font-semibold mb-4 text-center text-lg">🔔 Hi-Hat</h3>

      {/* Hi-Hat Preset Buttons */}
      <div className="mb-6">
        <h4 className="font-semibold mb-3 text-center">Quick Triggers</h4>
        <div className="grid grid-cols-2 gap-2">
          <button
            onClick={() => triggerHiHat("closed_default")}
            disabled={!isLoaded || !isPlaying}
            className="px-3 py-2 bg-gradient-to-r from-yellow-600 to-yellow-700 text-white rounded hover:from-yellow-700 hover:to-yellow-800 disabled:bg-gray-400 disabled:cursor-not-allowed text-sm font-semibold"
          >
            🔔 Closed
          </button>

          <button
            onClick={() => triggerHiHat("open_default")}
            disabled={!isLoaded || !isPlaying}
            className="px-3 py-2 bg-gradient-to-r from-yellow-500 to-yellow-600 text-white rounded hover:from-yellow-600 hover:to-yellow-700 disabled:bg-gray-400 disabled:cursor-not-allowed text-sm font-semibold"
          >
            🔔 Open
          </button>

          <button
            onClick={() => triggerHiHat("closed_tight")}
            disabled={!isLoaded || !isPlaying}
            className="px-3 py-2 bg-gradient-to-r from-amber-600 to-amber-700 text-white rounded hover:from-amber-700 hover:to-amber-800 disabled:bg-gray-400 disabled:cursor-not-allowed text-sm font-semibold"
          >
            🔔 Tight
          </button>

          <button
            onClick={() => triggerHiHat("open_bright")}
            disabled={!isLoaded || !isPlaying}
            className="px-3 py-2 bg-gradient-to-r from-yellow-400 to-yellow-500 text-white rounded hover:from-yellow-500 hover:to-yellow-600 disabled:bg-gray-400 disabled:cursor-not-allowed text-sm font-semibold"
          >
            🔔 Bright
          </button>

          <button
            onClick={() => triggerHiHat("closed_dark")}
            disabled={!isLoaded || !isPlaying}
            className="px-3 py-2 bg-gradient-to-r from-yellow-700 to-yellow-800 text-white rounded hover:from-yellow-800 hover:to-yellow-900 disabled:bg-gray-400 disabled:cursor-not-allowed text-sm font-semibold"
          >
            🔔 Dark
          </button>

          <button
            onClick={() => triggerHiHat("open_long")}
            disabled={!isLoaded || !isPlaying}
            className="px-3 py-2 bg-gradient-to-r from-amber-500 to-amber-600 text-white rounded hover:from-amber-600 hover:to-amber-700 disabled:bg-gray-400 disabled:cursor-not-allowed text-sm font-semibold"
          >
            🔔 Long
          </button>
        </div>
      </div>

      {/* Preset Selection Dropdown */}
      <div className="mb-4">
        <label className="block text-sm font-medium mb-2">Current Preset</label>
        <select
          value={hihatPreset}
          onChange={(e) => handleHihatPresetChange(e.target.value)}
          disabled={!isLoaded}
          className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded disabled:cursor-not-allowed"
        >
          <option value="closed_default">Closed Default</option>
          <option value="open_default">Open Default</option>
          <option value="closed_tight">Closed Tight</option>
          <option value="open_bright">Open Bright</option>
          <option value="closed_dark">Closed Dark</option>
          <option value="open_long">Open Long</option>
        </select>
      </div>

      {/* Hi-Hat Controls */}
      <div className="space-y-3">
        {/* Base Frequency */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Frequency</label>
          <input
            type="range"
            min="4000"
            max="16000"
            step="100"
            value={hihatConfig.baseFrequency}
            onChange={(e) =>
              handleHihatConfigChange(
                "baseFrequency",
                parseFloat(e.target.value)
              )
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-16 text-sm font-mono text-right">
            {hihatConfig.baseFrequency.toFixed(0)}Hz
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
            value={hihatConfig.volume}
            onChange={(e) =>
              handleHihatConfigChange("volume", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {hihatConfig.volume.toFixed(2)}
          </span>
        </div>

        {/* Decay Time */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Decay</label>
          <input
            type="range"
            min="0.01"
            max="3"
            step="0.01"
            value={hihatConfig.decayTime}
            onChange={(e) =>
              handleHihatConfigChange("decayTime", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {hihatConfig.decayTime.toFixed(2)}s
          </span>
        </div>

        {/* Brightness */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Brightness</label>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={hihatConfig.brightness}
            onChange={(e) =>
              handleHihatConfigChange("brightness", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {hihatConfig.brightness.toFixed(2)}
          </span>
        </div>

        {/* Resonance */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Resonance</label>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={hihatConfig.resonance}
            onChange={(e) =>
              handleHihatConfigChange("resonance", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {hihatConfig.resonance.toFixed(2)}
          </span>
        </div>

        {/* Attack Time */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Attack</label>
          <input
            type="range"
            min="0.001"
            max="0.1"
            step="0.001"
            value={hihatConfig.attackTime}
            onChange={(e) =>
              handleHihatConfigChange("attackTime", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {hihatConfig.attackTime.toFixed(3)}s
          </span>
        </div>

        {/* Open/Closed Toggle */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Type</label>
          <button
            onClick={() =>
              handleHihatConfigChange("isOpen", !hihatConfig.isOpen)
            }
            disabled={!isLoaded}
            className={`px-4 py-2 text-sm font-medium rounded transition-colors disabled:cursor-not-allowed ${
              hihatConfig.isOpen
                ? "bg-yellow-600 text-white hover:bg-yellow-700 disabled:bg-gray-600"
                : "bg-amber-600 text-white hover:bg-amber-700 disabled:bg-gray-600"
            }`}
          >
            {hihatConfig.isOpen ? "🔔 Open" : "🔔 Closed"}
          </button>
        </div>
      </div>

      {/* Release Button */}
      <button
        onClick={releaseHiHat}
        disabled={!isLoaded || !isPlaying}
        className="w-full mt-4 px-4 py-2 bg-gray-600 text-white rounded hover:bg-gray-500 disabled:bg-gray-700 disabled:cursor-not-allowed"
      >
        Release Hi-Hat
      </button>
    </div>
  );
}