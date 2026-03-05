use crate::ui::app::App;
use crate::ui::types::Popup;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap, Table, Row, Cell};
use ratatui::Frame;

use super::utils::centered_rect;

pub fn draw_popup(f: &mut Frame, app: &App, p: &Popup) {
    match p {
        Popup::Inspect { title, content } => {
            let area = centered_rect(80, 80, f.area());
            f.render_widget(Clear, area);
            let text: Text = content
                .lines()
                .map(|l| Line::from(Span::raw(l.to_string())))
                .collect::<Vec<_>>()
                .into();
            let w = Paragraph::new(text)
                .block(Block::default().borders(Borders::ALL).title(title.clone()))
                .wrap(Wrap { trim: false });
            f.render_widget(w, area);
        }
        Popup::ConfirmReset { name, .. } => {
            let area = centered_rect(60, 25, f.area());
            f.render_widget(Clear, area);
            let msg = format!(
                "RESET {name}?\nThis will STOP it, REMOVE it, and DELETE its volumes.\n\n[y/Enter]=Reset, [n/Esc]=Cancel"
            );
            let w = Paragraph::new(msg)
                .block(Block::default().borders(Borders::ALL).title(" ⚠️  RESET CONTAINER "))
                .wrap(Wrap { trim: false });
            f.render_widget(w, area);
        }
        Popup::ConfirmComposeRestart { infra_running } => {
            let area = centered_rect(60, 25, f.area());
            f.render_widget(Clear, area);
            let msg = if *infra_running {
                "Detected running stack containers.\n\n[r/Enter]=Restart services, [k]=Keep, [Esc]=Cancel".to_string()
            } else {
                "Start services now?\n\n[r/Enter]=docker compose up -d, [Esc]=Cancel".to_string()
            };
            let w = Paragraph::new(msg)
                .block(Block::default().borders(Borders::ALL).title(" Docker compose "))
                .wrap(Wrap { trim: false });
            f.render_widget(w, area);
        }
        Popup::ScaleService { name, input, .. } => {
            let area = centered_rect(50, 20, f.area());
            f.render_widget(Clear, area);
            let msg = format!("Scale service: {name}\nNew replicas: {input}█\n\n[Enter]:Confirm, [Esc]:Cancel");
            let w = Paragraph::new(msg)
                .block(Block::default().borders(Borders::ALL).title(" ⚖️ Scale Service "))
                .wrap(Wrap { trim: false });
            f.render_widget(w, area);
        }
        Popup::Volumes { volumes, selected } => {
            let area = centered_rect(80, 70, f.area());
            f.render_widget(Clear, area);
            let title = " 📂 Volumes Explorer ";

            let header_cells = ["Name", "Driver", "Size"]
                .iter()
                .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
            let header = Row::new(header_cells).height(1).bottom_margin(1);

            let rows = volumes.iter().enumerate().map(|(i, vol)| {
                let style = if i == *selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let cells = vec![
                    Cell::from(vol.name.clone()),
                    Cell::from(vol.driver.clone()),
                    Cell::from(vol.size.clone().unwrap_or_else(|| "-".to_string())),
                ];
                Row::new(cells).style(style)
            });

            let t = Table::new(
                rows,
                [
                    ratatui::layout::Constraint::Min(40),
                    ratatui::layout::Constraint::Length(15),
                    ratatui::layout::Constraint::Length(15),
                ],
            )
            .header(header)
            .block(Block::default().borders(Borders::ALL).title(title));

            f.render_widget(t, area);

            let help_area = ratatui::layout::Rect {
                x: area.x,
                y: area.y + area.height,
                width: area.width,
                height: 1,
            };
            let help_text = Paragraph::new(" ↑/↓:Nav  d:Rm  D:ForceRm  Esc/Enter:Close ").style(Style::default().fg(Color::Gray));
            f.render_widget(help_text, help_area);
        }
        Popup::Networks { networks, selected } => {
            let area = centered_rect(80, 70, f.area());
            f.render_widget(Clear, area);
            let title = " 🌐 Networks Explorer ";

            let header_cells = ["ID", "Name", "Driver", "Scope"]
                .iter()
                .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
            let header = Row::new(header_cells).height(1).bottom_margin(1);

            let rows = networks.iter().enumerate().map(|(i, net)| {
                let style = if i == *selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let cells = vec![
                    Cell::from(net.id.clone()),
                    Cell::from(net.name.clone()),
                    Cell::from(net.driver.clone()),
                    Cell::from(net.scope.clone()),
                ];
                Row::new(cells).style(style)
            });

            let t = Table::new(
                rows,
                [
                    ratatui::layout::Constraint::Length(15),
                    ratatui::layout::Constraint::Min(30),
                    ratatui::layout::Constraint::Length(15),
                    ratatui::layout::Constraint::Length(15),
                ],
            )
            .header(header)
            .block(Block::default().borders(Borders::ALL).title(title));

            f.render_widget(t, area);

            let help_area = ratatui::layout::Rect {
                x: area.x,
                y: area.y + area.height,
                width: area.width,
                height: 1,
            };
            let help_text = Paragraph::new(" ↑/↓:Nav  d:Rm  Esc/Enter:Close ").style(Style::default().fg(Color::Gray));
            f.render_widget(help_text, help_area);
        }
        Popup::ContextSwitch { contexts, selected } => {
            let area = centered_rect(60, 40, f.area());
            f.render_widget(Clear, area);
            let items: Vec<ListItem> = contexts
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let prefix = if c.current { "★ " } else { "  " };
                    let style = if i == *selected {
                        Style::default().fg(Color::Black).bg(Color::Rgb(0, 255, 255)).add_modifier(Modifier::BOLD)
                    } else if c.current {
                        Style::default().fg(Color::Rgb(255, 170, 0))
                    } else {
                        Style::default()
                    };
                    ListItem::new(format!("{prefix}{}", c.name)).style(style)
                })
                .collect();
            let w = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(" 🔌 Switch Docker Context "))
                .highlight_symbol("▶ ");
            f.render_widget(w, area);
        }
        Popup::SystemHealth { data } => {
            let area = centered_rect(65, 55, f.area());
            f.render_widget(Clear, area);
            let title = " 📊 Global System Health (Disk Usage) ";
            let mut lines = vec![
                Line::from(vec![
                    Span::styled("  TYPE", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw("            "),
                    Span::styled("TOTAL", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw("    "),
                    Span::styled("ACTIVE", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw("    "),
                    Span::styled("SIZE", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw("    "),
                    Span::styled("RECLAIMABLE", Style::default().add_modifier(Modifier::BOLD)),
                ]),
                Line::from("  ".to_string() + &"─".repeat(area.width.saturating_sub(6) as usize)),
            ];
            for row in data {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<15}", row.kind), Style::default().fg(Color::Rgb(255, 170, 0))),
                    Span::raw(format!("{:<10}", row.total)),
                    Span::raw(format!("{:<10}", row.active)),
                    Span::raw(format!("{:<10}", row.size)),
                    Span::styled(format!("{:<15}", row.reclaimable), Style::default().fg(Color::Green)),
                ]));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::raw("   [X] "),
                Span::styled("Run System Prune (Cleanup)", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw("   [Esc] "),
                Span::styled("Close", Style::default().fg(Color::White)),
            ]));
            let w = Paragraph::new(lines)
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan)).title(title))
                .wrap(Wrap { trim: false });
            f.render_widget(w, area);
        }
        Popup::ImageExplorer { images, selected } => {
            let area = centered_rect(80, 70, f.area());
            f.render_widget(Clear, area);
            let title = " 📦 Image Explorer ";

            let header_cells = ["ID", "Repository", "Tag", "Size", "Created"]
                .iter()
                .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
            let header = Row::new(header_cells).height(1).bottom_margin(1);

            let rows = images.iter().enumerate().map(|(i, img)| {
                let style = if i == *selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let cells = vec![
                    Cell::from(img.id.clone()),
                    Cell::from(img.repository.clone()),
                    Cell::from(img.tag.clone()),
                    Cell::from(img.size.clone()),
                    Cell::from(img.created_since.clone()),
                ];
                Row::new(cells).style(style)
            });

            let t = Table::new(
                rows,
                [
                    ratatui::layout::Constraint::Length(15),
                    ratatui::layout::Constraint::Min(30),
                    ratatui::layout::Constraint::Length(10),
                    ratatui::layout::Constraint::Length(10),
                    ratatui::layout::Constraint::Length(15),
                ],
            )
            .header(header)
            .block(Block::default().borders(Borders::ALL).title(title));

            f.render_widget(t, area);

            // Add help text at the bottom
            let help_area = ratatui::layout::Rect {
                x: area.x,
                y: area.y + area.height,
                width: area.width,
                height: 1,
            };
            let help_text = Paragraph::new(" ↑/↓:Nav  d:Rm  D:ForceRm  Esc/Enter:Close ").style(Style::default().fg(Color::Gray));
            f.render_widget(help_text, help_area);
        }
        Popup::ConfirmPrune => {
            let area = centered_rect(50, 30, f.area());
            f.render_widget(Clear, area);
            let msg = vec![
                Line::from(""),
                Line::from(vec![Span::styled("  ⚠️  SYSTEM PRUNE  ⚠️", Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD))]),
                Line::from(""),
                Line::from("  This will REMOVE:"),
                Line::from("  - all stopped containers"),
                Line::from("  - all unused networks"),
                Line::from("  - all dangling images"),
                Line::from("  - all dangling build cache"),
                Line::from(""),
                Line::from(vec![
                    Span::raw("  [y/Enter] "),
                    Span::styled("PRUNE EVERYTHING", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::raw("  [n/Esc] "),
                    Span::styled("Cancel", Style::default().fg(Color::White)),
                ]),
            ];
            let w = Paragraph::new(msg)
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)).title(" Dangerous Action "))
                .wrap(Wrap { trim: false });
            f.render_widget(w, area);
        }
        Popup::Help => {
            let area = centered_rect(70, 70, f.area());
            f.render_widget(Clear, area);
            let msg = "Raccourcis clavier :\n\n\
                Global :\n\
                - q / Ctrl+C : Quitter\n\
                - Tab : Changer le focus (Liste / Logs)\n\
                - ? : Afficher cette aide\n\
                - C : Changer de contexte Docker\n\
                - H : Dashboard de santé globale (Disk Usage)\n\
                - V : Lister les volumes\n\
                - N : Lister les réseaux\n\
                - / : Filtrer la liste\n\n\
                Navigation :\n\
                - Haut/Bas : Sélectionner un item\n\
                - Espace : Développer/Réduire un groupe\n\
                - v : (Dés)électionner pour action groupée\n\n\
                Actions sur Containers/Services :\n\
                - t : Démarrer\n\
                - s : Arrêter\n\
                - r : Redémarrer\n\
                - e : Shell interactif\n\
                - L : Logs multi-conteneurs (Compose)\n\
                - d : Supprimer\n\
                - i : Inspecter (JSON)\n\
                - S : Scaler le service Swarm\n\
                - o : Ouvrir dans le navigateur\n\
                - P : Épingler (Pin)\n\n\
                Logs & Maintenance :\n\
                - H : Dashboard de santé / Cleanup\n\
                - I : Explorateur d'images (Image Explorer)\n\
                - m : Mode Copie\n\
                - y : Copier tout le buffer\n\
                - PageUp/PageDown : Défiler";
            let w = Paragraph::new(msg)
                .block(Block::default().borders(Borders::ALL).title(" Aide des raccourcis "))
                .wrap(Wrap { trim: false });
            f.render_widget(w, area);
        }
    }
}
