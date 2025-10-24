"use client";

import React, { useRef, useState } from "react";
import init, {
  WasmStage,
  WasmKickDrum,
  WasmHiHat,
  WasmSnareDrum,
  WasmTomDrum,
} from "../public/wasm/libgooey.js";

import Sequencer from "./sequencer";
import Mixer from "./mixer";
import { SpectrumAnalyzerWithRef } from "./spectrum-analyzer";
import { SpectrogramDisplayWithRef } from "./spectrogram-display";
import OscillatorsTab from "./tabs/oscillators-tab";
import KickDrumTab from "./tabs/kick-drum-tab";
import HiHatTab from "./tabs/hihat-tab";
import SnareTab from "./tabs/snare-tab";
import TomTab from "./tabs/tom-tab";

export default function WasmTest() {
  const stageRef = useRef<WasmStage | null>(null);
  const kickDrumRef = useRef<WasmKickDrum | null>(null);
  const hihatRef = useRef<WasmHiHat | null>(null);
  const snareRef = useRef<WasmSnareDrum | null>(null);
  const tomRef = useRef<WasmTomDrum | null>(null);
  const audioContextRef = useRef<AudioContext | null>(null);
  const kickAudioSourceRef = useRef<AudioBufferSourceNode | null>(null);
  const hihatAudioSourceRef = useRef<AudioBufferSourceNode | null>(null);
  const snareAudioSourceRef = useRef<AudioBufferSourceNode | null>(null);
  const tomAudioSourceRef = useRef<AudioBufferSourceNode | null>(null);

  const spectrumAnalyzerRef = useRef<{
    connectSource: (source: AudioNode) => void;
    getAnalyser: () => AnalyserNode | null;
    getMonitoringNode: () => GainNode | null;
  } | null>(null);
  
  const spectrogramRef = useRef<{
    connectSource: (source: AudioNode) => void;
  } | null>(null);
  
  const audioProcessorRef = useRef<ScriptProcessorNode | null>(null);
  const [isLoaded, setIsLoaded] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [isPlaying, setIsPlaying] = useState(false);
  const [activeTab, setActiveTab] = useState<
    "oscillators" | "kick" | "hihat" | "snare" | "tom"
  >("kick");
  const [volumes, setVolumes] = useState([1.0, 1.0, 1.0, 1.0]); // Volume for each instrument
  const [frequencies, setFrequencies] = useState([200, 300, 440, 600]); // Frequency for each instrument
  const [modulatorFrequencies, setModulatorFrequencies] = useState([
    100, 150, 220, 300,
  ]); // Modulator frequency for each instrument (for ring modulation)
  const [waveforms, setWaveforms] = useState([1, 1, 1, 1]); // Waveform for each instrument (0=Sine, 1=Square, 2=Saw, 3=Triangle)
  const [enabled, setEnabled] = useState([false, false, false, false]); // Enabled state for each instrument - disabled by default
  const [adsrValues, setAdsrValues] = useState([
    { attack: 0.01, decay: 0.1, sustain: 0.7, release: 0.3 }, // Bass Drum
    { attack: 0.001, decay: 0.05, sustain: 0.3, release: 0.1 }, // Snare
    { attack: 0.001, decay: 0.02, sustain: 0.2, release: 0.05 }, // Hi-hat
    { attack: 0.005, decay: 0.2, sustain: 0.8, release: 0.5 }, // Cymbal
  ]);

  // Kick drum specific state
  const [kickPreset, setKickPreset] = useState("default");
  const [kickConfig, setKickConfig] = useState({
    frequency: 50.0,
    punch: 0.7,
    sub: 0.8,
    click: 0.3,
    decay: 0.8,
    pitchDrop: 0.6,
    volume: 0.6,
  });

  // Hi-hat specific state
  const [hihatPreset, setHihatPreset] = useState("closed_default");
  const [hihatConfig, setHihatConfig] = useState({
    baseFrequency: 8000.0,
    resonance: 0.7,
    brightness: 0.6,
    decayTime: 0.1,
    attackTime: 0.001,
    volume: 0.6,
    isOpen: false,
  });

  // Snare specific state
  const [snarePreset, setSnarePreset] = useState("default");
  const [snareConfig, setSnareConfig] = useState({
    frequency: 200.0,
    tonal: 0.4,
    noise: 0.7,
    crack: 0.5,
    decay: 0.15,
    pitchDrop: 0.3,
    volume: 0.6,
  });

  // Tom specific state
  const [tomPreset, setTomPreset] = useState("default");
  const [tomConfig, setTomConfig] = useState({
    frequency: 120.0,
    tonal: 0.8,
    punch: 0.4,
    decay: 0.4,
    pitchDrop: 0.3,
    volume: 0.6,
  });

  // Keyboard mapping state
  const [pressedKeys, setPressedKeys] = useState<Set<string>>(new Set());
  const [keyboardEnabled, setKeyboardEnabled] = useState(true);
  
  // Saturation control state
  const [saturation, setSaturation] = useState(0.0);

  // Keyboard mapping configuration
  const keyMappings = {
    a: {
      name: "Kick Drum",
      action: () => triggerKickDrum(),
      color: "bg-red-500",
      emoji: "🥁",
    },
    s: {
      name: "Snare Drum",
      action: () => triggerSnareDrum(),
      color: "bg-orange-500",
      emoji: "🥁",
    },
    d: {
      name: "Hi-Hat",
      action: () => triggerHiHat(),
      color: "bg-yellow-500",
      emoji: "🔔",
    },
    f: {
      name: "Tom Drum",
      action: () => triggerTomDrum(),
      color: "bg-purple-500",
      emoji: "🥁",
    },
  };

  // Keyboard event handlers
  React.useEffect(() => {
    if (!keyboardEnabled) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      const key = event.key.toLowerCase();

      // Prevent default behavior for our mapped keys
      if (key in keyMappings) {
        event.preventDefault();

        // Only trigger if not already pressed (prevent key repeat)
        if (!pressedKeys.has(key)) {
          setPressedKeys((prev) => new Set(prev).add(key));
          keyMappings[key as keyof typeof keyMappings].action();
          console.log(
            `Keyboard trigger: ${key.toUpperCase()} -> ${
              keyMappings[key as keyof typeof keyMappings].name
            }`
          );
        }
      }
    };

    const handleKeyUp = (event: KeyboardEvent) => {
      const key = event.key.toLowerCase();

      if (key in keyMappings) {
        event.preventDefault();
        setPressedKeys((prev) => {
          const newSet = new Set(prev);
          newSet.delete(key);
          return newSet;
        });
      }
    };

    // Add global event listeners
    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("keyup", handleKeyUp);

    // Cleanup
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("keyup", handleKeyUp);
    };
  }, [keyboardEnabled, pressedKeys, isLoaded, isPlaying]);

  async function loadWasm() {
    setIsLoading(true);
    try {
      // Initialize the WASM module
      await init();

      // Create stage instance with 44100 sample rate
      stageRef.current = new WasmStage(44100);

      // Add multiple oscillators with different frequencies
      stageRef.current.add_oscillator(44100, 200); // Bass drum
      stageRef.current.add_oscillator(44100, 300); // Snare
      stageRef.current.add_oscillator(44100, 440); // Hi-hat
      stageRef.current.add_oscillator(44100, 600); // Cymbal

      // Initialize ADSR settings for each instrument
      adsrValues.forEach((adsr, index) => {
        stageRef.current?.set_instrument_adsr(
          index,
          adsr.attack,
          adsr.decay,
          adsr.sustain,
          adsr.release
        );
      });

      // Initialize modulator frequencies for each instrument
      modulatorFrequencies.forEach((freq, index) => {
        stageRef.current?.set_instrument_modulator_frequency(index, freq);
      });

      // Initialize enabled state for each instrument (basic oscillators disabled by default)
      enabled.forEach((isEnabled, index) => {
        stageRef.current?.set_instrument_enabled(index, isEnabled);
      });

      // Create kick drum instance
      kickDrumRef.current = new WasmKickDrum(44100);
      kickDrumRef.current.set_volume(kickConfig.volume);

      // Create hi-hat instance
      hihatRef.current = WasmHiHat.new_with_preset(44100, "closed_default");
      hihatRef.current.set_volume(hihatConfig.volume);

      // Create snare instance
      snareRef.current = new WasmSnareDrum(44100);
      snareRef.current.set_volume(snareConfig.volume);

      // Create tom instance
      tomRef.current = new WasmTomDrum(44100);
      tomRef.current.set_volume(tomConfig.volume);

      // Initialize Web Audio API
      audioContextRef.current = new AudioContext();

      setIsLoaded(true);
      console.log(
        "WASM Stage with 4 oscillators, kick drum, hi-hat, snare, tom, and Web Audio loaded successfully!"
      );
    } catch (error) {
      console.error("Failed to load WASM:", error);
      alert("Failed to load WASM module: " + String(error));
    } finally {
      setIsLoading(false);
    }
  }

  async function startAudio() {
    if (!audioContextRef.current || !stageRef.current) {
      alert('WASM not loaded yet. Click "Load WASM" first.');
      return;
    }

    try {
      // Resume audio context if suspended
      if (audioContextRef.current.state === "suspended") {
        await audioContextRef.current.resume();
      }
      
      // Create a continuous audio processing loop using ScriptProcessorNode
      // This is needed for the sequencer to advance through steps
      const bufferSize = 4096;
      const processor = audioContextRef.current.createScriptProcessor(bufferSize, 0, 1);
      audioProcessorRef.current = processor;
      
      processor.onaudioprocess = (event) => {
        if (!stageRef.current || !audioContextRef.current) return;
        
        const outputBuffer = event.outputBuffer;
        const outputData = outputBuffer.getChannelData(0);
        
        // Process audio samples continuously
        for (let i = 0; i < outputBuffer.length; i++) {
          const currentTime = audioContextRef.current.currentTime + (i / audioContextRef.current.sampleRate);
          outputData[i] = stageRef.current.tick(currentTime);
        }
      };
      
      // Connect the processor to the audio graph
      if (spectrumAnalyzerRef.current && spectrumAnalyzerRef.current.getMonitoringNode()) {
        processor.connect(spectrumAnalyzerRef.current.getMonitoringNode()!);
      } else {
        processor.connect(audioContextRef.current.destination);
      }
      
      setIsPlaying(true);
      console.log('Audio started with continuous processing!');
    } catch (error) {
      console.error("Failed to start audio:", error);
      alert("Failed to start audio");
    }
  }

  function stopAudio() {
    // Disconnect the audio processor
    if (audioProcessorRef.current) {
      audioProcessorRef.current.disconnect();
      audioProcessorRef.current = null;
    }
    
    setIsPlaying(false);
    console.log("Audio stopped!");
  }

  function triggerAll() {
    if (!audioContextRef.current || !stageRef.current || !isPlaying) {
      alert('Audio not started yet. Click "Start Audio" first.');
      return;
    }

    try {
      // Trigger all instruments in the stage
      stageRef.current.trigger_all();

      console.log("All instruments triggered!");
    } catch (error) {
      console.error("Failed to trigger instruments:", error);
      alert("Failed to trigger instruments");
    }
  }

  function triggerInstrument(index: number, name: string) {
    if (!audioContextRef.current || !stageRef.current || !isPlaying) {
      alert('Audio not started yet. Click "Start Audio" first.');
      return;
    }

    try {
      // Trigger specific instrument in the stage
      stageRef.current.trigger_instrument(index);

      console.log(`${name} triggered!`);
    } catch (error) {
      console.error(`Failed to trigger ${name}:`, error);
      alert(`Failed to trigger ${name}`);
    }
  }

  function handleVolumeChange(index: number, volume: number) {
    if (!stageRef.current) return;

    // Update the WASM stage
    stageRef.current.set_instrument_volume(index, volume);

    // Update local state for UI
    setVolumes((prev) => {
      const newVolumes = [...prev];
      newVolumes[index] = volume;
      return newVolumes;
    });
  }

  function handleFrequencyChange(index: number, frequency: number) {
    if (!stageRef.current) return;

    // Update the WASM stage
    stageRef.current.set_instrument_frequency(index, frequency);

    // Update local state for UI
    setFrequencies((prev) => {
      const newFrequencies = [...prev];
      newFrequencies[index] = frequency;
      return newFrequencies;
    });
  }

  function handleWaveformChange(index: number, waveformType: number) {
    if (!stageRef.current) return;

    // Update the WASM stage
    stageRef.current.set_instrument_waveform(index, waveformType);

    // Update local state for UI
    setWaveforms((prev) => {
      const newWaveforms = [...prev];
      newWaveforms[index] = waveformType;
      return newWaveforms;
    });
  }

  function handleAdsrChange(
    index: number,
    param: "attack" | "decay" | "sustain" | "release",
    value: number
  ) {
    if (!stageRef.current) return;

    // Update local state for UI
    setAdsrValues((prev) => {
      const newAdsrValues = [...prev];
      newAdsrValues[index] = { ...newAdsrValues[index], [param]: value };

      // Update the WASM stage with new ADSR values
      const adsr = newAdsrValues[index];
      if (stageRef.current) {
        stageRef.current.set_instrument_adsr(
          index,
          adsr.attack,
          adsr.decay,
          adsr.sustain,
          adsr.release
        );
      }

      return newAdsrValues;
    });
  }

  function handleModulatorFrequencyChange(index: number, frequency: number) {
    if (!stageRef.current) return;

    // Update the WASM stage
    stageRef.current.set_instrument_modulator_frequency(index, frequency);

    // Update local state for UI
    setModulatorFrequencies((prev) => {
      const newModulatorFrequencies = [...prev];
      newModulatorFrequencies[index] = frequency;
      return newModulatorFrequencies;
    });
  }

  function handleEnabledChange(index: number, isEnabled: boolean) {
    if (!stageRef.current) return;

    // Update the WASM stage
    stageRef.current.set_instrument_enabled(index, isEnabled);

    // Update local state for UI
    setEnabled((prev) => {
      const newEnabled = [...prev];
      newEnabled[index] = isEnabled;
      return newEnabled;
    });
  }

  function releaseInstrument(index: number, name: string) {
    if (!audioContextRef.current || !stageRef.current) {
      alert('Audio not started yet. Click "Start Audio" first.');
      return;
    }

    try {
      stageRef.current.release_instrument(index);
      console.log(`${name} released!`);
    } catch (error) {
      console.error(`Failed to release ${name}:`, error);
      alert(`Failed to release ${name}`);
    }
  }

  function releaseAll() {
    if (!audioContextRef.current || !stageRef.current) {
      alert('Audio not started yet. Click "Start Audio" first.');
      return;
    }

    try {
      stageRef.current.release_all();
      console.log("All instruments released!");
    } catch (error) {
      console.error("Failed to release all instruments:", error);
      alert("Failed to release all instruments");
    }
  }

  // Kick drum functions
  function triggerKickDrum() {
    if (!audioContextRef.current || !stageRef.current || !isPlaying) {
      alert('Audio not started yet. Click "Start Audio" first.');
      return;
    }

    try {
      // Trigger kick drum using the stage
      stageRef.current.trigger_kick();

      console.log("Kick drum triggered!");
    } catch (error) {
      console.error("Failed to trigger kick drum:", error);
      alert("Failed to trigger kick drum");
    }
  }

  function releaseKickDrum() {
    if (!audioContextRef.current || !stageRef.current) {
      alert('Audio not started yet. Click "Start Audio" first.');
      return;
    }

    try {
      console.log("Kick drum released!");
    } catch (error) {
      console.error("Failed to release kick drum:", error);
      alert("Failed to release kick drum");
    }
  }

  function handleKickConfigChange(
    param: keyof typeof kickConfig,
    value: number
  ) {
    if (!kickDrumRef.current) return;

    console.log(`[DEBUG] handleKickConfigChange called: param=${param}, value=${value}`);

    // Update local state
    setKickConfig((prev) => ({ ...prev, [param]: value }));

    // Update the kick drum
    switch (param) {
      case "frequency":
        kickDrumRef.current.set_frequency(value);
        break;
      case "punch":
        kickDrumRef.current.set_punch(value);
        break;
      case "sub":
        kickDrumRef.current.set_sub(value);
        break;
      case "click":
        kickDrumRef.current.set_click(value);
        break;
      case "decay":
        kickDrumRef.current.set_decay(value);
        break;
      case "pitchDrop":
        kickDrumRef.current.set_pitch_drop(value);
        break;
      case "volume":
        console.log(`[DEBUG] Setting kick volume to ${value}`);
        if (stageRef.current) {
          stageRef.current.set_instrument_volume(0, value);
          // Update stage's internal kick drum volume (used by sequencer)
          stageRef.current.set_kick_config(
            kickConfig.frequency,
            kickConfig.punch,
            kickConfig.sub,
            kickConfig.click,
            kickConfig.decay,
            kickConfig.pitchDrop,
            value
          );
        }
        if (kickDrumRef.current) {
          kickDrumRef.current.set_volume(value);
        }
        break;
    }
  }

  function handleKickPresetChange(preset: string) {
    if (!stageRef.current) return;

    setKickPreset(preset);

    // Load preset using the stage
    stageRef.current.load_kick_preset(preset);

    // Update state to match preset values
    switch (preset) {
      case "punchy":
        setKickConfig({
          frequency: 60.0,
          punch: 0.9,
          sub: 0.6,
          click: 0.4,
          decay: 0.6,
          pitchDrop: 0.7,
          volume: 0.85,
        });
        break;
      case "deep":
        setKickConfig({
          frequency: 45.0,
          punch: 0.5,
          sub: 1.0,
          click: 0.2,
          decay: 1.2,
          pitchDrop: 0.5,
          volume: 0.9,
        });
        break;
      case "tight":
        setKickConfig({
          frequency: 70.0,
          punch: 0.8,
          sub: 0.7,
          click: 0.5,
          decay: 0.4,
          pitchDrop: 0.8,
          volume: 0.6,
        });
        break;
      default: // default
        setKickConfig({
          frequency: 50.0,
          punch: 0.7,
          sub: 0.8,
          click: 0.3,
          decay: 0.8,
          pitchDrop: 0.6,
          volume: 0.6,
        });
    }
  }

  // Hi-hat functions
  function triggerHiHat(preset?: string) {
    if (!audioContextRef.current || !stageRef.current || !isPlaying) {
      alert('Audio not started yet. Click "Start Audio" first.');
      return;
    }

    try {
      // Change preset if provided
      if (preset && preset !== hihatPreset) {
        stageRef.current.load_hihat_preset(preset);
        setHihatPreset(preset);
        updateHihatConfigFromPreset(preset);
      }

      // Trigger hi-hat using the stage
      stageRef.current.trigger_hihat();

      console.log(`Hi-hat ${preset || hihatPreset} triggered!`);
    } catch (error) {
      console.error("Failed to trigger hi-hat:", error);
      alert("Failed to trigger hi-hat");
    }
  }

  function releaseHiHat() {
    if (!audioContextRef.current || !stageRef.current) {
      alert('Audio not started yet. Click "Start Audio" first.');
      return;
    }

    try {
      console.log("Hi-hat released!");
    } catch (error) {
      console.error("Failed to release hi-hat:", error);
      alert("Failed to release hi-hat");
    }
  }

  function updateHihatConfigFromPreset(preset: string) {
    // Update state to match preset values based on HiHatConfig presets
    switch (preset) {
      case "closed_default":
        setHihatConfig({
          baseFrequency: 8000.0,
          resonance: 0.7,
          brightness: 0.6,
          decayTime: 0.1,
          attackTime: 0.001,
          volume: 0.6,
          isOpen: false,
        });
        break;
      case "open_default":
        setHihatConfig({
          baseFrequency: 8000.0,
          resonance: 0.5,
          brightness: 0.8,
          decayTime: 0.8,
          attackTime: 0.001,
          volume: 0.7,
          isOpen: true,
        });
        break;
      case "closed_tight":
        setHihatConfig({
          baseFrequency: 10000.0,
          resonance: 0.8,
          brightness: 0.5,
          decayTime: 0.05,
          attackTime: 0.001,
          volume: 0.9,
          isOpen: false,
        });
        break;
      case "open_bright":
        setHihatConfig({
          baseFrequency: 12000.0,
          resonance: 0.4,
          brightness: 1.0,
          decayTime: 1.2,
          attackTime: 0.001,
          volume: 0.6,
          isOpen: true,
        });
        break;
      case "closed_dark":
        setHihatConfig({
          baseFrequency: 6000.0,
          resonance: 0.6,
          brightness: 0.3,
          decayTime: 0.15,
          attackTime: 0.002,
          volume: 0.7,
          isOpen: false,
        });
        break;
      case "open_long":
        setHihatConfig({
          baseFrequency: 7000.0,
          resonance: 0.3,
          brightness: 0.7,
          decayTime: 2.0,
          attackTime: 0.001,
          volume: 0.6,
          isOpen: true,
        });
        break;
      default:
        setHihatConfig({
          baseFrequency: 8000.0,
          resonance: 0.7,
          brightness: 0.6,
          decayTime: 0.1,
          attackTime: 0.001,
          volume: 0.6,
          isOpen: false,
        });
    }
  }

  function handleHihatConfigChange(
    param: keyof typeof hihatConfig,
    value: number | boolean
  ) {
    if (!hihatRef.current) return;

    console.log(`[DEBUG] handleHihatConfigChange called: param=${param}, value=${value}`);

    // Update local state
    setHihatConfig((prev) => ({ ...prev, [param]: value }));

    // Update the hi-hat
    switch (param) {
      case "baseFrequency":
        hihatRef.current.set_frequency(value as number);
        break;
      case "resonance":
        hihatRef.current.set_resonance(value as number);
        break;
      case "brightness":
        hihatRef.current.set_brightness(value as number);
        break;
      case "decayTime":
        hihatRef.current.set_decay(value as number);
        break;
      case "attackTime":
        hihatRef.current.set_attack(value as number);
        break;
      case "volume":
        console.log(`[DEBUG] Setting hihat volume to ${value}`);
        if (stageRef.current) {
          stageRef.current.set_instrument_volume(2, value as number);
          // Update stage's internal hihat drum volume (used by sequencer)
          stageRef.current.set_hihat_config(
            hihatConfig.baseFrequency,
            hihatConfig.resonance,
            hihatConfig.brightness,
            hihatConfig.decayTime,
            hihatConfig.attackTime,
            value as number,
            hihatConfig.isOpen
          );
        }
        if (hihatRef.current) {
          hihatRef.current.set_volume(value as number);
        }
        break;
      case "isOpen":
        hihatRef.current.set_open(value as boolean);
        break;
    }
  }

  function handleHihatPresetChange(preset: string) {
    if (!hihatRef.current) return;

    setHihatPreset(preset);

    // Create new hi-hat with preset
    hihatRef.current = WasmHiHat.new_with_preset(44100, preset);

    // Update state to match preset values
    updateHihatConfigFromPreset(preset);
  }

  // Snare drum functions
  function triggerSnareDrum() {
    if (!audioContextRef.current || !stageRef.current || !isPlaying) {
      alert('Audio not started yet. Click "Start Audio" first.');
      return;
    }

    try {
      // Trigger snare drum using the stage
      stageRef.current.trigger_snare();

      console.log("Snare drum triggered!");
    } catch (error) {
      console.error("Failed to trigger snare drum:", error);
      alert("Failed to trigger snare drum");
    }
  }

  function releaseSnareDrum() {
    if (!audioContextRef.current || !stageRef.current) {
      alert('Audio not started yet. Click "Start Audio" first.');
      return;
    }

    try {
      console.log("Snare drum released!");
    } catch (error) {
      console.error("Failed to release snare drum:", error);
      alert("Failed to release snare drum");
    }
  }

  function handleSnareConfigChange(
    param: keyof typeof snareConfig,
    value: number
  ) {
    if (!snareRef.current) return;

    console.log(`[DEBUG] handleSnareConfigChange called: param=${param}, value=${value}`);

    // Update local state
    setSnareConfig((prev) => ({ ...prev, [param]: value }));

    // Update the snare drum
    switch (param) {
      case "frequency":
        snareRef.current.set_frequency(value);
        break;
      case "tonal":
        snareRef.current.set_tonal(value);
        break;
      case "noise":
        snareRef.current.set_noise(value);
        break;
      case "crack":
        snareRef.current.set_crack(value);
        break;
      case "decay":
        snareRef.current.set_decay(value);
        break;
      case "pitchDrop":
        snareRef.current.set_pitch_drop(value);
        break;
      case "volume":
        console.log(`[DEBUG] Setting snare volume to ${value}`);
        if (stageRef.current) {
          stageRef.current.set_instrument_volume(1, value);
          // Update stage's internal snare drum volume (used by sequencer)
          stageRef.current.set_snare_config(
            snareConfig.frequency,
            snareConfig.tonal,
            snareConfig.noise,
            snareConfig.crack,
            snareConfig.decay,
            snareConfig.pitchDrop,
            value
          );
        }
        if (snareRef.current) {
          snareRef.current.set_volume(value);
        }
        break;
    }
  }

  function handleSnarePresetChange(preset: string) {
    if (!snareRef.current) return;

    setSnarePreset(preset);

    // Create new snare drum with preset
    snareRef.current = WasmSnareDrum.new_with_preset(44100, preset);

    // Update state to match preset values
    switch (preset) {
      case "crispy":
        setSnareConfig({
          frequency: 250.0,
          tonal: 0.3,
          noise: 0.8,
          crack: 0.7,
          decay: 0.12,
          pitchDrop: 0.4,
          volume: 0.85,
        });
        break;
      case "deep":
        setSnareConfig({
          frequency: 180.0,
          tonal: 0.6,
          noise: 0.6,
          crack: 0.3,
          decay: 0.2,
          pitchDrop: 0.2,
          volume: 0.9,
        });
        break;
      case "tight":
        setSnareConfig({
          frequency: 220.0,
          tonal: 0.3,
          noise: 0.8,
          crack: 0.8,
          decay: 0.08,
          pitchDrop: 0.5,
          volume: 0.6,
        });
        break;
      case "fat":
        setSnareConfig({
          frequency: 160.0,
          tonal: 0.7,
          noise: 0.5,
          crack: 0.4,
          decay: 0.25,
          pitchDrop: 0.1,
          volume: 0.9,
        });
        break;
      default: // default
        setSnareConfig({
          frequency: 200.0,
          tonal: 0.4,
          noise: 0.7,
          crack: 0.5,
          decay: 0.15,
          pitchDrop: 0.3,
          volume: 0.6,
        });
    }
  }

  // Tom drum functions
  function triggerTomDrum() {
    if (!audioContextRef.current || !stageRef.current || !isPlaying) {
      alert('Audio not started yet. Click "Start Audio" first.');
      return;
    }

    try {
      // Trigger tom drum using the stage
      stageRef.current.trigger_tom();

      console.log("Tom drum triggered!");
    } catch (error) {
      console.error("Failed to trigger tom drum:", error);
      alert("Failed to trigger tom drum");
    }
  }

  function releaseTomDrum() {
    if (!audioContextRef.current || !stageRef.current) {
      alert('Audio not started yet. Click "Start Audio" first.');
      return;
    }

    try {
      console.log("Tom drum released!");
    } catch (error) {
      console.error("Failed to release tom drum:", error);
      alert("Failed to release tom drum");
    }
  }

  function handleTomConfigChange(param: keyof typeof tomConfig, value: number) {
    if (!tomRef.current) return;

    console.log(`[DEBUG] handleTomConfigChange called: param=${param}, value=${value}`);

    // Update local state
    setTomConfig((prev) => ({ ...prev, [param]: value }));

    // Update the tom drum
    switch (param) {
      case "frequency":
        tomRef.current.set_frequency(value);
        break;
      case "tonal":
        tomRef.current.set_tonal(value);
        break;
      case "punch":
        tomRef.current.set_punch(value);
        break;
      case "decay":
        tomRef.current.set_decay(value);
        break;
      case "pitchDrop":
        tomRef.current.set_pitch_drop(value);
        break;
      case "volume":
        console.log(`[DEBUG] Setting tom volume to ${value}`);
        if (stageRef.current) {
          stageRef.current.set_instrument_volume(3, value);
          // Update stage's internal tom drum volume (used by sequencer)
          stageRef.current.set_tom_config(
            tomConfig.frequency,
            tomConfig.tonal,
            tomConfig.punch,
            tomConfig.decay,
            tomConfig.pitchDrop,
            value
          );
        }
        if (tomRef.current) {
          tomRef.current.set_volume(value);
        }
        break;
    }
  }

  function handleTomPresetChange(preset: string) {
    if (!tomRef.current) return;

    setTomPreset(preset);

    // Create new tom drum with preset
    tomRef.current = WasmTomDrum.new_with_preset(44100, preset);

    // Update state to match preset values
    switch (preset) {
      case "high_tom":
        setTomConfig({
          frequency: 180.0,
          tonal: 0.9,
          punch: 0.5,
          decay: 0.3,
          pitchDrop: 0.4,
          volume: 0.85,
        });
        break;
      case "mid_tom":
        setTomConfig({
          frequency: 120.0,
          tonal: 0.8,
          punch: 0.4,
          decay: 0.4,
          pitchDrop: 0.3,
          volume: 0.6,
        });
        break;
      case "low_tom":
        setTomConfig({
          frequency: 90.0,
          tonal: 0.7,
          punch: 0.3,
          decay: 0.6,
          pitchDrop: 0.2,
          volume: 0.85,
        });
        break;
      case "floor_tom":
        setTomConfig({
          frequency: 70.0,
          tonal: 0.6,
          punch: 0.2,
          decay: 0.8,
          pitchDrop: 0.15,
          volume: 0.9,
        });
        break;
      default: // default
        setTomConfig({
          frequency: 120.0,
          tonal: 0.8,
          punch: 0.4,
          decay: 0.4,
          pitchDrop: 0.3,
          volume: 0.6,
        });
    }
  }

  function handleSaturationChange(value: number) {
    if (!stageRef.current) return;
    
    setSaturation(value);
    stageRef.current.set_saturation(value);
  }

  return (
    <div className="p-8 max-w-7xl mx-auto">
      <h1 className="text-2xl font-bold mb-6 text-center">
        WASM Audio Engine Test
      </h1>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-8">
        {/* Left Column - Controls */}
        <div className="space-y-4">
          <button
            onClick={loadWasm}
            disabled={isLoading || isLoaded}
            className="w-full px-4 py-2 bg-blue-500 text-white rounded hover:bg-blue-600 disabled:bg-gray-400 disabled:cursor-not-allowed"
          >
            {isLoading
              ? "Loading..."
              : isLoaded
              ? "Audio Engine Loaded (Stage + Kick)"
              : "Load Audio Engine"}
          </button>

          <button
            onClick={isPlaying ? stopAudio : startAudio}
            disabled={!isLoaded}
            className="w-full px-4 py-2 bg-green-500 text-white rounded hover:bg-green-600 disabled:bg-gray-400 disabled:cursor-not-allowed"
          >
            {isPlaying ? "Stop Audio" : "Start Audio"}
          </button>

          {/* Saturation Control */}
          <div className="border border-gray-600 rounded p-3 bg-gray-800">
            <h3 className="font-semibold text-lg mb-2">🎛️ Master Saturation</h3>
            <div className="flex items-center space-x-3">
              <label className="text-white font-medium min-w-fit">Saturation:</label>
              <input
                type="range"
                min="0"
                max="1"
                step="0.01"
                value={saturation}
                onChange={(e) => handleSaturationChange(parseFloat(e.target.value))}
                className="flex-1"
                disabled={!isLoaded}
              />
              <span className="text-white font-mono min-w-fit">{saturation.toFixed(2)}</span>
            </div>
            <p className="text-xs text-gray-400 mt-1">
              Adds harmonic distortion to the final audio output
            </p>
          </div>

          {/* Keyboard Mapping Widget */}
          <div className="border-t pt-4">
            <div className="flex items-center justify-between mb-3">
              <h3 className="font-semibold text-lg">⌨️ Keyboard Mapping</h3>
              <button
                onClick={() => setKeyboardEnabled(!keyboardEnabled)}
                className={`px-3 py-1 text-sm font-medium rounded transition-colors ${
                  keyboardEnabled
                    ? "bg-green-600 text-white hover:bg-green-700"
                    : "bg-red-600 text-white hover:bg-red-700"
                }`}
              >
                {keyboardEnabled ? "🔊 ON" : "🔇 OFF"}
              </button>
            </div>

            <div className="grid grid-cols-2 gap-3 mb-4">
              {Object.entries(keyMappings).map(([key, mapping]) => (
                <div
                  key={key}
                  className={`p-3 rounded-lg border-2 transition-all duration-150 ${
                    pressedKeys.has(key)
                      ? `${mapping.color} border-white scale-95 shadow-lg`
                      : "bg-gray-800 border-gray-600 hover:border-gray-500"
                  }`}
                >
                  <div className="flex items-center justify-between">
                    <div className="flex items-center space-x-2">
                      <span className="text-2xl">{mapping.emoji}</span>
                      <div>
                        <div className="text-sm font-medium">
                          {mapping.name}
                        </div>
                        <div className="text-xs text-gray-400">
                          Press {key.toUpperCase()}
                        </div>
                      </div>
                    </div>
                    <div
                      className={`w-8 h-8 rounded-lg flex items-center justify-center font-bold text-sm transition-colors ${
                        pressedKeys.has(key)
                          ? "bg-white text-gray-900"
                          : "bg-gray-700 text-white"
                      }`}
                    >
                      {key.toUpperCase()}
                    </div>
                  </div>
                </div>
              ))}
            </div>

            <div className="text-xs text-gray-400 mb-2">
              💡 Click anywhere on the page to ensure keyboard focus, then press
              A, S, D, or F to trigger instruments
            </div>
          </div>

          {/* Tab Navigation */}
          <div className="border-t pt-4">
            <div className="flex space-x-1 mb-6">
              <button
                onClick={() => setActiveTab("oscillators")}
                className={`px-4 py-2 rounded-t-lg font-medium transition-colors ${
                  activeTab === "oscillators"
                    ? "bg-blue-600 text-white border-b-2 border-blue-600"
                    : "bg-gray-700 text-gray-300 hover:bg-gray-600"
                }`}
              >
                🎵 Oscillators
              </button>
              <button
                onClick={() => setActiveTab("kick")}
                className={`px-4 py-2 rounded-t-lg font-medium transition-colors ${
                  activeTab === "kick"
                    ? "bg-red-600 text-white border-b-2 border-red-600"
                    : "bg-gray-700 text-gray-300 hover:bg-gray-600"
                }`}
              >
                🥁 Kick Drum
              </button>
              <button
                onClick={() => setActiveTab("hihat")}
                className={`px-4 py-2 rounded-t-lg font-medium transition-colors ${
                  activeTab === "hihat"
                    ? "bg-yellow-600 text-white border-b-2 border-yellow-600"
                    : "bg-gray-700 text-gray-300 hover:bg-gray-600"
                }`}
              >
                🔔 Hi-Hat
              </button>
              <button
                onClick={() => setActiveTab("snare")}
                className={`px-4 py-2 rounded-t-lg font-medium transition-colors ${
                  activeTab === "snare"
                    ? "bg-orange-600 text-white border-b-2 border-orange-600"
                    : "bg-gray-700 text-gray-300 hover:bg-gray-600"
                }`}
              >
                🥁 Snare
              </button>
              <button
                onClick={() => setActiveTab("tom")}
                className={`px-4 py-2 rounded-t-lg font-medium transition-colors ${
                  activeTab === "tom"
                    ? "bg-purple-600 text-white border-b-2 border-purple-600"
                    : "bg-gray-700 text-gray-300 hover:bg-gray-600"
                }`}
              >
                🥁 Tom
              </button>
            </div>

            {/* Tab Content */}
            {activeTab === "oscillators" && (
              <OscillatorsTab
                isLoaded={isLoaded}
                isPlaying={isPlaying}
                volumes={volumes}
                frequencies={frequencies}
                modulatorFrequencies={modulatorFrequencies}
                waveforms={waveforms}
                enabled={enabled}
                adsrValues={adsrValues}
                triggerAll={triggerAll}
                triggerInstrument={triggerInstrument}
                handleVolumeChange={handleVolumeChange}
                handleFrequencyChange={handleFrequencyChange}
                handleWaveformChange={handleWaveformChange}
                handleAdsrChange={handleAdsrChange}
                handleModulatorFrequencyChange={handleModulatorFrequencyChange}
                handleEnabledChange={handleEnabledChange}
                releaseInstrument={releaseInstrument}
                releaseAll={releaseAll}
              />
            )}

            {/* Kick Drum Tab */}
            {activeTab === "kick" && (
              <KickDrumTab
                isLoaded={isLoaded}
                isPlaying={isPlaying}
                kickPreset={kickPreset}
                kickConfig={kickConfig}
                triggerKickDrum={triggerKickDrum}
                releaseKickDrum={releaseKickDrum}
                handleKickConfigChange={handleKickConfigChange}
                handleKickPresetChange={handleKickPresetChange}
              />
            )}


            {/* Hi-Hat Tab */}
            {activeTab === "hihat" && (
              <HiHatTab
                isLoaded={isLoaded}
                isPlaying={isPlaying}
                hihatPreset={hihatPreset}
                hihatConfig={hihatConfig}
                triggerHiHat={triggerHiHat}
                releaseHiHat={releaseHiHat}
                handleHihatConfigChange={handleHihatConfigChange}
                handleHihatPresetChange={handleHihatPresetChange}
              />
            )}

            {/* Snare Tab */}
            {activeTab === "snare" && (
              <SnareTab
                isLoaded={isLoaded}
                isPlaying={isPlaying}
                snarePreset={snarePreset}
                snareConfig={snareConfig}
                triggerSnareDrum={triggerSnareDrum}
                releaseSnareDrum={releaseSnareDrum}
                handleSnareConfigChange={handleSnareConfigChange}
                handleSnarePresetChange={handleSnarePresetChange}
              />
            )}

            {/* Tom Tab */}
            {activeTab === "tom" && (
              <TomTab
                isLoaded={isLoaded}
                isPlaying={isPlaying}
                tomPreset={tomPreset}
                tomConfig={tomConfig}
                triggerTomDrum={triggerTomDrum}
                releaseTomDrum={releaseTomDrum}
                handleTomConfigChange={handleTomConfigChange}
                handleTomPresetChange={handleTomPresetChange}
              />
            )}

          </div>
        </div>

        {/* Right Column - Spectrum Analyzer and Status */}
        <div className="space-y-4">
          {/* Spectrum Analyzer */}
          <div>
            <SpectrumAnalyzerWithRef
              ref={spectrumAnalyzerRef}
              audioContext={audioContextRef.current}
              isActive={isPlaying}
              width={600}
              height={200}
            />
          </div>

          {/* Sequencer */}
          <Sequencer stage={stageRef.current} isPlaying={isPlaying} />

          {/* Mixer */}
          <Mixer
            kickVolume={kickConfig.volume}
            snareVolume={snareConfig.volume}
            hihatVolume={hihatConfig.volume}
            tomVolume={tomConfig.volume}
            onKickVolumeChange={(volume) => handleKickConfigChange('volume', volume)}
            onSnareVolumeChange={(volume) => handleSnareConfigChange('volume', volume)}
            onHihatVolumeChange={(volume) => handleHihatConfigChange('volume', volume)}
            onTomVolumeChange={(volume) => handleTomConfigChange('volume', volume)}
            isLoaded={isLoaded}
          />

          {/* Spectrogram Display */}
          <div>
            <SpectrogramDisplayWithRef
              ref={spectrogramRef}
              audioContext={audioContextRef.current}
              isActive={isPlaying}
              width={600}
              height={200}
              analyser={spectrumAnalyzerRef.current?.getAnalyser() || null}
            />
          </div>

          
          <div className="p-4 bg-gray-800 rounded">
            <h2 className="font-semibold mb-2">Status:</h2>
            <p>
              WASM Stage:{" "}
              {isLoaded ? "✅ Loaded (4 oscillators)" : "❌ Not loaded"}
            </p>
            <p>
              Kick Drum:{" "}
              {isLoaded && kickDrumRef.current ? "✅ Loaded" : "❌ Not loaded"}
            </p>
            <p>
              Hi-Hat:{" "}
              {isLoaded && hihatRef.current ? "✅ Loaded" : "❌ Not loaded"}
            </p>
            <p>
              Snare:{" "}
              {isLoaded && snareRef.current ? "✅ Loaded" : "❌ Not loaded"}
            </p>
            <p>
              Tom: {isLoaded && tomRef.current ? "✅ Loaded" : "❌ Not loaded"}
            </p>
            <p>
              Audio Context: {audioContextRef.current ? "✅ Ready" : "❌ No"}
            </p>
            <p>Audio Playing: {isPlaying ? "✅ Yes" : "❌ No"}</p>
          </div>
        </div>
      </div>

      <div className="mt-4 p-4 bg-blue-900/20 border border-blue-600/30 rounded">
        <h3 className="font-semibold mb-2 text-blue-300">Engine API Demo:</h3>
        <ul className="text-sm space-y-1 text-blue-100">
          <li>
            • <strong>Multi-instrument Stage</strong>: 4 oscillators with
            independent controls
          </li>
          <li>
            • <strong>Individual control</strong>: Trigger each instrument
            separately
          </li>
          <li>
            • <strong>Group control</strong>: Trigger all instruments
            simultaneously
          </li>
          <li>
            • <strong>Enable/disable</strong>: Toggle instruments on/off to
            mute/unmute individual instruments
          </li>
          <li>
            • <strong>Volume control</strong>: Adjust volume (0.0-1.0) for each
            instrument
          </li>
          <li>
            • <strong>Frequency control</strong>: Adjust frequency (50-2000Hz)
            for each instrument
          </li>
          <li>
            • <strong>Waveform control</strong>: Select waveform type (Sine,
            Square, Saw, Triangle, Ring Mod) for each instrument
          </li>
          <li>
            • <strong>Ring modulation</strong>: Modulator frequency control for
            Ring Mod waveform
          </li>
          <li>
            • <strong>ADSR envelope</strong>: Real-time Attack, Decay, Sustain,
            Release control per instrument
          </li>
          <li>
            • <strong>Release control</strong>: Manually trigger release phase
            for individual or all instruments
          </li>
          <li>
            • <strong>Kick Drum Instrument</strong>: Comprehensive 3-layer kick
            drum with sub-bass, punch, and click layers
          </li>
          <li>
            • <strong>Kick Presets</strong>: Built-in presets (Default, Punchy,
            Deep, Tight) for different kick styles
          </li>
          <li>
            • <strong>Kick Parameters</strong>: Frequency, punch, sub-bass,
            click, decay time, and pitch drop controls
          </li>
          <li>
            • <strong>Hi-Hat Instrument</strong>: Noise-based hi-hat with dual
            oscillators and envelope control
          </li>
          <li>
            • <strong>Hi-Hat Presets</strong>: 6 built-in presets (Closed
            Default, Open Default, Closed Tight, Open Bright, Closed Dark, Open
            Long)
          </li>
          <li>
            • <strong>Hi-Hat Parameters</strong>: Base frequency, resonance,
            brightness, decay time, attack time, volume, and open/closed mode
          </li>
          <li>
            • <strong>Snare Instrument</strong>: Comprehensive 3-layer snare
            drum with tonal, noise, and crack components
          </li>
          <li>
            • <strong>Snare Presets</strong>: Built-in presets (Default, Crispy,
            Deep, Tight, Fat) for different snare styles
          </li>
          <li>
            • <strong>Snare Parameters</strong>: Frequency, tonal amount, noise
            amount, crack amount, decay time, and pitch drop controls
          </li>
          <li>
            • <strong>Tom Instrument</strong>: Comprehensive tom drum with tonal
            and punch layers for realistic drum sounds
          </li>
          <li>
            • <strong>Tom Presets</strong>: Built-in presets (Default, High Tom,
            Mid Tom, Low Tom, Floor Tom) for different tom styles
          </li>
          <li>
            • <strong>Tom Parameters</strong>: Frequency, tonal amount, punch
            amount, decay time, and pitch drop controls
          </li>
          <li>
            • <strong>Audio mixing</strong>: Stage.tick() sums all instrument
            outputs with controls applied
          </li>
        </ul>
      </div>

      <div className="mt-4 p-4 bg-yellow-900/20 border border-yellow-600/30 rounded">
        <h3 className="font-semibold mb-2 text-yellow-300">Instructions:</h3>
        <ol className="list-decimal list-inside text-sm space-y-1 text-yellow-100">
          <li>
            Click "Load Audio Engine" to initialize the WASM Stage with 4
            oscillators, kick drum, hi-hat, snare, and tom
          </li>
          <li>Click "Start Audio" to begin audio processing</li>
          <li>
            <strong>🎹 Keyboard Mapping:</strong> Use keyboard shortcuts for
            quick testing:
            <ul className="list-disc list-inside ml-4 text-xs space-y-0.5 text-yellow-200 mt-1">
              <li>
                <strong>A</strong> → Trigger Kick Drum
              </li>
              <li>
                <strong>S</strong> → Trigger Snare Drum
              </li>
              <li>
                <strong>D</strong> → Trigger Hi-Hat
              </li>
              <li>
                <strong>F</strong> → Trigger Cymbal
              </li>
              <li>Toggle keyboard mapping on/off with the ON/OFF button</li>
              <li>Visual feedback shows which keys are currently pressed</li>
            </ul>
          </li>
          <li>Use individual instrument buttons to test single oscillators</li>
          <li>Adjust instrument controls for each oscillator:</li>
          <ul className="list-disc list-inside ml-4 text-xs space-y-0.5 text-yellow-200">
            <li>
              <strong>Enable/Disable:</strong> Click ON/OFF button to
              mute/unmute individual instruments
            </li>
            <li>
              <strong>Volume:</strong> Control relative volume of each
              instrument (0.0-1.0)
            </li>
            <li>
              <strong>Frequency:</strong> Change the pitch of each instrument
              (50-2000Hz)
            </li>
            <li>
              <strong>Waveform:</strong> Select tone quality (Sine, Square, Saw,
              Triangle, Ring Mod)
            </li>
            <li>
              <strong>Modulator:</strong> Control modulator frequency for Ring
              Mod waveform (50-2000Hz)
            </li>
          </ul>
          <li>Adjust ADSR envelope controls to shape the sound envelope:</li>
          <ul className="list-disc list-inside ml-4 text-xs space-y-0.5 text-yellow-200">
            <li>
              <strong>Attack:</strong> Time to reach full volume (0.001-2s)
            </li>
            <li>
              <strong>Decay:</strong> Time to drop to sustain level (0.001-2s)
            </li>
            <li>
              <strong>Sustain:</strong> Level held while triggered (0-1)
            </li>
            <li>
              <strong>Release:</strong> Time to fade to silence (0.001-5s)
            </li>
          </ul>
          <li>Use "Release" buttons to manually trigger the release phase</li>
          <li>Use "Release All" to release all instruments simultaneously</li>
          <li>
            Use "Trigger All" to hear the mixed output of all instruments with
            all controls applied
          </li>
          <li>
            Test the comprehensive kick drum with its own dedicated section:
          </li>
          <ul className="list-disc list-inside ml-4 text-xs space-y-0.5 text-yellow-200">
            <li>
              <strong>Presets:</strong> Try different kick styles (Default,
              Punchy, Deep, Tight)
            </li>
            <li>
              <strong>Frequency:</strong> Adjust fundamental frequency
              (20-200Hz)
            </li>
            <li>
              <strong>Punch:</strong> Control mid-range impact layer
            </li>
            <li>
              <strong>Sub Bass:</strong> Control low-end presence
            </li>
            <li>
              <strong>Click:</strong> Control high-frequency transient
            </li>
            <li>
              <strong>Decay:</strong> Adjust overall decay time
            </li>
            <li>
              <strong>Pitch Drop:</strong> Control frequency sweep effect
            </li>
          </ul>
          <li>Test the hi-hat instrument with its dedicated section:</li>
          <ul className="list-disc list-inside ml-4 text-xs space-y-0.5 text-yellow-200">
            <li>
              <strong>Quick Triggers:</strong> Use preset buttons for instant
              testing (Closed, Open, Tight, Bright, Dark, Long)
            </li>
            <li>
              <strong>Preset Selection:</strong> Choose from 6 different hi-hat
              styles in the dropdown
            </li>
            <li>
              <strong>Base Frequency:</strong> Adjust the fundamental frequency
              (4000-16000Hz)
            </li>
            <li>
              <strong>Brightness:</strong> Control high-frequency emphasis and
              transient sharpness
            </li>
            <li>
              <strong>Resonance:</strong> Adjust metallic character and filter
              resonance
            </li>
            <li>
              <strong>Decay Time:</strong> Control how long the hi-hat rings out
            </li>
            <li>
              <strong>Attack Time:</strong> Adjust the initial transient speed
            </li>
            <li>
              <strong>Open/Closed Toggle:</strong> Switch between open and
              closed hi-hat modes
            </li>
          </ul>
          <li>Test the snare drum instrument with its dedicated section:</li>
          <ul className="list-disc list-inside ml-4 text-xs space-y-0.5 text-yellow-200">
            <li>
              <strong>Presets:</strong> Try different snare styles (Default,
              Crispy, Deep, Tight, Fat)
            </li>
            <li>
              <strong>Frequency:</strong> Adjust fundamental frequency
              (100-600Hz)
            </li>
            <li>
              <strong>Tonal Amount:</strong> Control the body and pitch
              component of the snare
            </li>
            <li>
              <strong>Noise Amount:</strong> Control the main snare noise
              character
            </li>
            <li>
              <strong>Crack Amount:</strong> Control high-frequency snap and
              crack
            </li>
            <li>
              <strong>Decay Time:</strong> Adjust overall decay time (0.01-2s)
            </li>
            <li>
              <strong>Pitch Drop:</strong> Control frequency sweep effect for
              realistic sound
            </li>
          </ul>
          <li>Test the tom drum instrument with its dedicated section:</li>
          <ul className="list-disc list-inside ml-4 text-xs space-y-0.5 text-yellow-200">
            <li>
              <strong>Presets:</strong> Try different tom styles (Default, High
              Tom, Mid Tom, Low Tom, Floor Tom)
            </li>
            <li>
              <strong>Frequency:</strong> Adjust fundamental frequency
              (60-400Hz)
            </li>
            <li>
              <strong>Tonal Amount:</strong> Control the body and pitch
              component of the tom
            </li>
            <li>
              <strong>Punch Amount:</strong> Control mid-range impact and attack
              character
            </li>
            <li>
              <strong>Decay Time:</strong> Adjust overall decay time (0.05-3s)
            </li>
            <li>
              <strong>Pitch Drop:</strong> Control frequency sweep effect for
              realistic tom sound
            </li>
          </ul>
        </ol>
      </div>
    </div>
  );
}
