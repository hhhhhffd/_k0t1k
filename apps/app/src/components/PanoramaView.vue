<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'

/**
 * The rotating title-screen panorama of a Minecraft version, drawn like the game does: the camera sits in a cube
 * of six pictures and turns slowly. The faces come from InventivetalentDev/minecraft-assets, which keeps each
 * version's assets on its own branch (served with CORS, so WebGL may use them).
 */
const props = defineProps<{ version: string }>()
const emit = defineEmits<{ failed: [] }>()

const BASE = 'https://raw.githubusercontent.com/InventivetalentDev/minecraft-assets'
const faceUrl = (version: string, i: number) =>
  `${BASE}/${encodeURIComponent(version)}/assets/minecraft/textures/gui/title/background/panorama_${i}.png`

/** Seconds for a full turn, and how far the camera looks down. */
const TURN_SECS = 180
const PITCH = (10 * Math.PI) / 180
const FOV_Y = (60 * Math.PI) / 180

const VERTEX = `
attribute vec2 pos;
varying vec2 ndc;
void main() { ndc = pos; gl_Position = vec4(pos, 0.0, 1.0); }`

// Faces in the game's order: 0 front (-z), 1 right (+x), 2 back (+z), 3 left (-x), 4 top (+y), 5 bottom (-y).
const FRAGMENT = `
precision mediump float;
varying vec2 ndc;
uniform sampler2D f0, f1, f2, f3, f4, f5;
uniform float yaw, pitch, tanX, tanY;
void main() {
  vec3 d = normalize(vec3(ndc.x * tanX, ndc.y * tanY, -1.0));
  float cp = cos(pitch), sp = sin(pitch);
  d = vec3(d.x, d.y * cp - d.z * sp, d.y * sp + d.z * cp);
  float cy = cos(yaw), sy = sin(yaw);
  d = vec3(d.x * cy - d.z * sy, d.y, d.x * sy + d.z * cy);
  vec3 a = abs(d);
  vec4 c;
  if (a.x >= a.y && a.x >= a.z) {
    vec2 uv = vec2(sign(d.x) * d.z, -d.y) / a.x * 0.5 + 0.5;
    c = d.x > 0.0 ? texture2D(f1, uv) : texture2D(f3, uv);
  } else if (a.y >= a.z) {
    // the top picture's lower edge and the bottom picture's upper edge meet the front
    if (d.y > 0.0) c = texture2D(f4, vec2(d.x, -d.z) / a.y * 0.5 + 0.5);
    else c = texture2D(f5, vec2(d.x, d.z) / a.y * 0.5 + 0.5);
  } else {
    vec2 uv = vec2(-sign(d.z) * d.x, -d.y) / a.z * 0.5 + 0.5;
    c = d.z < 0.0 ? texture2D(f0, uv) : texture2D(f2, uv);
  }
  gl_FragColor = c;
}`

const canvas = ref<HTMLCanvasElement | null>(null)
const ready = ref(false)
let gl: WebGLRenderingContext | null = null
let program: WebGLProgram | null = null
let frame = 0
let observer: ResizeObserver | null = null
const startedAt = performance.now()

function compile(type: number, source: string) {
  const shader = gl!.createShader(type)!
  gl!.shaderSource(shader, source)
  gl!.compileShader(shader)
  if (!gl!.getShaderParameter(shader, gl!.COMPILE_STATUS)) throw new Error(gl!.getShaderInfoLog(shader) ?? 'shader error')
  return shader
}

function setup(): boolean {
  gl = canvas.value?.getContext('webgl', { antialias: false, alpha: false }) ?? null
  if (!gl) return false
  program = gl.createProgram()!
  gl.attachShader(program, compile(gl.VERTEX_SHADER, VERTEX))
  gl.attachShader(program, compile(gl.FRAGMENT_SHADER, FRAGMENT))
  gl.linkProgram(program)
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program) ?? 'link error')
  gl.useProgram(program)
  gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer())
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW)
  const pos = gl.getAttribLocation(program, 'pos')
  gl.enableVertexAttribArray(pos)
  gl.vertexAttribPointer(pos, 2, gl.FLOAT, false, 0, 0)
  for (let i = 0; i < 6; i++) gl.uniform1i(gl.getUniformLocation(program, `f${i}`), i)
  return true
}

function loadImage(url: string) {
  return new Promise<HTMLImageElement>((resolve, reject) => {
    const img = new Image()
    img.crossOrigin = 'anonymous'
    img.onload = () => resolve(img)
    img.onerror = () => reject(new Error(`cannot load ${url}`))
    img.src = url
  })
}

/** Loads all six faces before showing anything, so the cube never appears with holes. */
async function load(version: string) {
  ready.value = false
  let images: HTMLImageElement[]
  try {
    images = await Promise.all([0, 1, 2, 3, 4, 5].map((i) => loadImage(faceUrl(version, i))))
  } catch (e) {
    if (version !== props.version) return
    console.warn(`no panorama for Minecraft ${version}`, e)
    emit('failed')
    return
  }
  if (version !== props.version || !gl) return
  images.forEach((img, i) => {
    gl!.activeTexture(gl!.TEXTURE0 + i)
    gl!.bindTexture(gl!.TEXTURE_2D, gl!.createTexture())
    gl!.texImage2D(gl!.TEXTURE_2D, 0, gl!.RGBA, gl!.RGBA, gl!.UNSIGNED_BYTE, img)
    gl!.texParameteri(gl!.TEXTURE_2D, gl!.TEXTURE_MIN_FILTER, gl!.LINEAR)
    gl!.texParameteri(gl!.TEXTURE_2D, gl!.TEXTURE_MAG_FILTER, gl!.LINEAR)
    gl!.texParameteri(gl!.TEXTURE_2D, gl!.TEXTURE_WRAP_S, gl!.CLAMP_TO_EDGE)
    gl!.texParameteri(gl!.TEXTURE_2D, gl!.TEXTURE_WRAP_T, gl!.CLAMP_TO_EDGE)
  })
  ready.value = true
}

function draw(now: number) {
  if (!gl || !program || !canvas.value) return
  // lite mode turns all motion off: the panorama stays still
  const still = document.documentElement.dataset.lite === 'true'
  if (ready.value) {
    const { width, height } = canvas.value
    gl.viewport(0, 0, width, height)
    const tanY = Math.tan(FOV_Y / 2)
    gl.uniform1f(gl.getUniformLocation(program, 'tanY'), tanY)
    gl.uniform1f(gl.getUniformLocation(program, 'tanX'), (tanY * width) / Math.max(height, 1))
    gl.uniform1f(gl.getUniformLocation(program, 'pitch'), -PITCH)
    gl.uniform1f(gl.getUniformLocation(program, 'yaw'), still ? 0 : (((now - startedAt) / 1000 / TURN_SECS) * Math.PI * 2) % (Math.PI * 2))
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4)
  }
  frame = requestAnimationFrame(draw)
}

onMounted(() => {
  try {
    if (!setup()) throw new Error('WebGL is unavailable')
  } catch (e) {
    console.warn('cannot draw the panorama', e)
    emit('failed')
    return
  }
  observer = new ResizeObserver(([entry]) => {
    if (!entry || !canvas.value) return
    const scale = window.devicePixelRatio || 1
    canvas.value.width = Math.round(entry.contentRect.width * scale)
    canvas.value.height = Math.round(entry.contentRect.height * scale)
  })
  observer.observe(canvas.value!)
  watch(() => props.version, (version) => void load(version), { immediate: true })
  frame = requestAnimationFrame(draw)
})
onBeforeUnmount(() => {
  cancelAnimationFrame(frame)
  observer?.disconnect()
  gl?.getExtension('WEBGL_lose_context')?.loseContext()
})
</script>

<template>
  <canvas ref="canvas" class="absolute inset-0 size-full transition-opacity duration-700" :class="ready ? 'opacity-100' : 'opacity-0'" />
</template>
