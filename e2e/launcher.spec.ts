import { test, expect } from '@playwright/test'

test('browser preview renders navigation and honestly disables native operations', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', e => errors.push(e.message))
  await page.goto('/')
  await expect(page.getByRole('img', { name: 'CatCL · CCL' })).toBeVisible()
  await expect(page.getByRole('heading', { name: '你的冒险基地' })).toBeVisible()
  await expect(page.getByText('浏览器预览', { exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: '创建实例', exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: '最小化窗口' })).toBeDisabled()
  await expect(page.getByRole('button', { name: '关闭窗口', exact: true })).toBeDisabled()
  await page.screenshot({ path: 'artifacts/home.png', fullPage: true })
  await page.getByRole('button', { name: '版本管理' }).click()
  await expect(page.getByRole('heading', { name: '还没有版本信息' })).toBeVisible()
  await page.getByRole('button', { name: '下载任务' }).click()
  await expect(page.getByRole('heading', { name: '没有进行中的下载' })).toBeVisible()
  await page.getByRole('button', { name: '启动器设置' }).click()
  await page.getByLabel('Java 可执行文件', { exact: true }).fill('C:\\Java\\bin\\java.exe')
  await expect(page.getByRole('button', { name: '检测', exact: true })).toBeDisabled()
  await expect(page.getByRole('button', { name: '保存设置' })).toBeDisabled()
  await expect(page.getByRole('button', { name: /自动安装/ })).toBeDisabled()
  await page.screenshot({ path: 'artifacts/settings.png', fullPage: true })
  expect(errors).toEqual([])
})

test('minimum desktop width has no horizontal overflow', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 600 })
  await page.goto('/')
  await expect(page.locator('.hero')).toHaveCSS('height', '225px')
  await expect(page.locator('.launcher')).toHaveCSS('border-radius', '50px')
  await page.screenshot({ path: 'artifacts/figma-home-800.png', fullPage: true })
  await page.getByRole('button', { name: '版本管理', exact: true }).click()
  await expect(page.getByRole('tab', { name: '正式版' })).toHaveAttribute('aria-selected', 'true')
  await page.getByRole('tab', { name: '快照' }).click()
  await expect(page.getByRole('tab', { name: '快照' })).toHaveAttribute('aria-selected', 'true')
  for (const navigation of ['开始游戏', '版本管理', '资源广场', '世界管理', '下载任务', '启动器设置']) {
    await page.getByRole('button', { name: navigation, exact: true }).click()
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
    await expect(page.getByRole('button', { name: '关闭窗口', exact: true })).toBeVisible()
  }
})

test('world manager renames, filters and imports through the desktop bridge', async ({ page }) => {
  await page.addInitScript(() => {
    const worlds = [{ id: 'saves/island', name: '小岛', folder: 'island', location: '公共存档', modifiedAt: 1700000000000 }]
    Object.assign(window, { isTauri: true, __TAURI_INTERNALS__: { invoke: async (command: string, args: Record<string, string>) => {
      if (command === 'load_settings') return { javaPath: 'java', offlineName: 'Player', memoryMb: 4096, gameDir: 'D:\\Minecraft', selectedVersion: '1.21.1', showSnapshots: false, instances: [{ name: '测试实例', version: '1.21.1', path: 'D:\\test-worlds' }], selectedInstance: '测试实例' }
      if (command === 'installed_versions' || command === 'list_versions') return []
      if (command === 'list_worlds') return { worlds: structuredClone(worlds), warnings: [] }
      if (command === 'rename_world') { worlds.find(w => w.id === args.id)!.name = args.name; return }
      if (command === 'import_world') { worlds.push({ id: 'saves/imported', name: args.name!, folder: 'imported', location: '公共存档', modifiedAt: 1700000000000 }); return }
      throw new Error(`Unexpected command: ${command}`)
    } } })
  })
  await page.setViewportSize({ width: 800, height: 600 })
  await page.goto('/')
  await page.getByRole('button', { name: '世界管理', exact: true }).click()
  await expect(page.getByRole('heading', { name: '小岛', exact: true })).toBeVisible()
  await page.screenshot({ path: 'artifacts/world-manager-800.png', fullPage: true })
  await page.getByRole('button', { name: '重命名 小岛' }).click()
  await expect(page.getByRole('dialog')).toBeVisible()
  await expect(page.getByLabel('世界显示名称')).toBeFocused()
  await page.getByLabel('世界显示名称').fill('')
  await page.getByRole('button', { name: '保存', exact: true }).click()
  await expect(page.getByRole('dialog').getByRole('alert')).toContainText('1–80')
  await page.getByLabel('世界显示名称').fill('我们的家')
  await page.getByRole('button', { name: '保存', exact: true }).click()
  await expect(page.getByRole('heading', { name: '我们的家', exact: true })).toBeVisible()
  await page.getByRole('button', { name: '开始游戏', exact: true }).click()
  await page.getByRole('button', { name: '世界管理', exact: true }).click()
  await expect(page.getByRole('heading', { name: '我们的家', exact: true })).toBeVisible()
  await page.getByLabel('搜索世界').fill('没有的世界')
  await expect(page.getByRole('heading', { name: '没有匹配的世界' })).toBeVisible()
  await page.getByLabel('搜索世界').fill('')
  await page.getByRole('button', { name: '导入世界', exact: true }).click()
  await page.getByLabel('世界显示名称').fill('新地图')
  await page.getByLabel('已解压的世界文件夹', { exact: true }).fill('D:\\downloads\\map')
  await page.getByRole('button', { name: '保存', exact: true }).click()
  await expect(page.getByRole('heading', { name: '新地图', exact: true })).toBeVisible()
  await expect(page.getByRole('heading', { name: '我们的家', exact: true })).toBeVisible()
})
