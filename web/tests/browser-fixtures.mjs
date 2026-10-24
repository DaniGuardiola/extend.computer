import { test as base } from '@playwright/test'

// Tests with multiple viewports/accounts need additional isolated contexts.
// Keep their lifecycle and failure artifacts here rather than in every suite.
export const test = base.extend({
  newContext: async ({ browser }, use) => {
    const contexts = []
    await use(async (options = {}) => {
      const context = await browser.newContext(options)
      contexts.push(context)
      return context
    })
    for (const context of contexts) {
      await context.close()
    }
  },
  newPage: async ({ newContext }, use) => {
    await use(async (options = {}) => (await newContext(options)).newPage())
  },
})
