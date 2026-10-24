import * as React from 'react'
import {
  Body,
  Button,
  Container,
  Column,
  Head,
  Heading,
  Hr,
  Html,
  Img,
  Link,
  Preview,
  Row,
  Section,
  Text,
  Tailwind,
  pixelBasedPreset,
} from 'react-email'

export type EmailKind = 'verify' | 'reset'

export const accountEmailCopy = {
  verify: {
    title: 'Verify your email',
    preview: 'One quick step to secure your extend.computer account.',
    description:
      'Confirm your email address so you can recover your account if you ever need to.',
    action: 'Verify email address',
    ignore:
      'If you didn’t create an extend.computer account, you can safely ignore this email.',
  },
  reset: {
    title: 'Reset your password',
    preview: 'Choose a new password for your extend.computer account.',
    description:
      'We received a request to reset your password. Use the button below to choose a new one.',
    action: 'Reset password',
    ignore:
      'If you didn’t request a password reset, you can safely ignore this email. Your password will stay the same.',
  },
} satisfies Record<EmailKind, Record<string, string>>

export function AccountEmail({
  kind,
  link,
  origin,
}: {
  kind: EmailKind
  link: string
  origin: string
}) {
  const copy = accountEmailCopy[kind]
  return (
    <Tailwind
      config={{
        presets: [pixelBasedPreset],
        theme: {
          extend: {
            colors: {
              canvas: '#f3f5ef',
              ink: '#20271e',
              line: '#dce2d5',
              brand: '#141714',
              wordmark: '#f1f3e9',
              acid: '#c2f269',
              muted: '#596451',
              'button-ink': '#182014',
              note: '#f3f6ef',
              'note-ink': '#35442d',
              rule: '#e2e7dc',
              link: '#3f6b16',
              footer: '#68735f',
            },
            fontFamily: { email: ['Arial', 'Helvetica', 'sans-serif'] },
          },
        },
      }}
    >
      <Html lang="en">
        <Head>
          <meta name="viewport" content="width=device-width, initial-scale=1" />
          <meta name="color-scheme" content="light dark" />
          <meta name="supported-color-schemes" content="light dark" />
          <style>{`@media only screen and (max-width: 600px) {
          .email-shell > tbody > tr > td { padding: 24px 12px !important; }
          .email-content > tbody > tr > td { padding: 32px 24px !important; }
          .email-brand > tbody > tr > td { padding: 24px !important; }
          .email-heading { font-size: 28px !important; }
        }
        @media (prefers-color-scheme: dark) {
          body, .email-shell, .email-shell > tbody > tr > td { background-color: #141714 !important; color: #f1f3e9 !important; }
          .email-card { background-color: #1c201c !important; border-color: #353b33 !important; }
          .email-heading, .email-copy { color: #f1f3e9 !important; }
          .email-muted, .email-footer, .email-footer a { color: #a3aa9e !important; }
          .email-note { background-color: #20271e !important; }
          .email-note-text { color: #bec9b2 !important; }
          .email-rule { border-color: #353b33 !important; }
          .email-fallback { color: #c2f269 !important; }
          .email-button { background-color: #c2f269 !important; color: #182014 !important; }
        }`}</style>
        </Head>
        <Preview>{copy.preview}</Preview>
        <Body className="email-body bg-canvas text-ink font-email m-0">
          <Section className="email-shell py-12 px-5">
            <Container className="email-card max-w-[560px] bg-white border border-solid border-line rounded-[12px] overflow-hidden">
              <Section className="email-brand bg-brand py-7 px-10 rounded-t-[11px]">
                <Row>
                  <Column className="w-11 align-middle">
                    <Link
                      href={origin}
                      aria-label="extend.computer home"
                      data-skip-in-text="true"
                    >
                      <Img
                        src={`${origin}/email-logo.png`}
                        alt=""
                        width="28"
                        height="32"
                        className="block"
                      />
                    </Link>
                  </Column>
                  <Column className="align-middle">
                    <Link
                      href={origin}
                      className="text-wordmark text-[23px] font-bold tracking-[-0.8px] no-underline"
                    >
                      extend<span className="text-acid">.</span>computer
                    </Link>
                  </Column>
                </Row>
              </Section>
              <Section className="email-content pt-9 px-10 pb-10">
                <Text className="email-muted text-muted text-[11px] font-bold tracking-[1.8px] m-0 mb-3">
                  YOUR ACCOUNT
                </Text>
                <Heading
                  className="email-heading text-[32px] leading-[1.2] font-bold tracking-[-1px] m-0 mb-5"
                  as="h1"
                >
                  {copy.title}
                </Heading>
                <Text className="email-copy text-[16px] leading-[26px] m-0">
                  {copy.description}
                </Text>
                <Section className="mt-7 mb-5 mx-0">
                  <Button
                    className="email-button bg-acid text-button-ink rounded-[6px] py-4 px-6 text-[15px] font-bold no-underline"
                    href={link}
                  >
                    {copy.action}
                  </Button>
                </Section>
                <Text className="email-muted text-muted text-[13px] leading-[21px] m-0">
                  This link expires in 30 minutes.
                </Text>
                {kind === 'reset' && (
                  <Section className="email-note bg-note py-4 px-5 rounded-[6px] mt-6">
                    <Text className="email-note-text text-note-ink text-[13px] leading-[21px] m-0">
                      Resetting your password signs you out of all sessions.
                      Two-factor authentication stays enabled. Your passkeys and
                      security keys are kept.
                    </Text>
                  </Section>
                )}
                <Hr className="email-rule border-rule mt-7 mb-6 mx-0" />
                <Text className="email-muted text-muted text-[13px] leading-[21px] m-0">
                  {copy.ignore}
                </Text>
                <Text className="email-muted text-muted text-[13px] leading-[21px] mt-5 mb-2 mx-0">
                  Button not working? Copy and paste this link into your
                  browser:
                </Text>
                <Link
                  className="email-fallback text-link text-[12px] leading-[20px] underline [word-break:break-all]"
                  href={link}
                >
                  {link.replace(/^https?:\/\//, '')}
                </Link>
              </Section>
            </Container>
            <Text className="email-footer text-footer text-[12px] leading-[20px] text-center mt-6 mb-0 mx-0">
              <Link href={origin} className="text-footer underline">
                extend.computer
              </Link>
              {' · Your devices, working together.'}
            </Text>
          </Section>
        </Body>
      </Html>
    </Tailwind>
  )
}
