# React Setup

`Interactive Prototype` 默认用单文件 inline React + Babel。

## 固定脚本

把这三段放进 HTML 的 `<head>`：

```html
<script src="https://unpkg.com/react@18.3.1/umd/react.development.js" integrity="sha384-hD6/rw4ppMLGNu3tX5cjIb+uRZ7UkRJ6BPkLpg4hAu/6onKUg4lLsHAs9EBPT82L" crossorigin="anonymous"></script>
<script src="https://unpkg.com/react-dom@18.3.1/umd/react-dom.development.js" integrity="sha384-u6aeetuaXnQ38mYT8rp6sbXaQe3NL9t+IBXmnYxwkUI2Hw4bsp2Wvmx4yRQF1uAm" crossorigin="anonymous"></script>
<script src="https://unpkg.com/@babel/standalone@7.29.0/babel.min.js" integrity="sha384-m08KidiNqLdpJqLq95G/LEi8Qvjl/xUYll3QILypMoQ65QorJ9Lvtp2RXYGBFj1y" crossorigin="anonymous"></script>
```

## 为什么默认单文件

- 本地 `file://` 可以直接打开
- 更适合交付 demo
- 避免“还要起服务”的额外摩擦

## 技术红线

### 1. 不要写通用 `styles`

错误：

```jsx
const styles = { ... };
```

正确：

```jsx
const settingsScreenStyles = { ... };
```

### 2. 多段 script 不共享 scope

如果拆多个 `type="text/babel"` 段，要把组件挂到 `window`：

```jsx
Object.assign(window, { BrowserWindow, IosFrame });
```

### 3. 不要用 `scrollIntoView`

它很容易搞坏容器内滚动。优先用：

```js
container.scrollTo({ top: y, behavior: 'smooth' });
```

## 单文件骨架

```html
<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Prototype</title>
  <!-- React scripts -->
</head>
<body>
  <div id="root"></div>
  <script type="text/babel">
    function App() {
      return <div>Hello prototype</div>;
    }
    ReactDOM.createRoot(document.getElementById('root')).render(<App />);
  </script>
</body>
</html>
```
