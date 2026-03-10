use crate::ui::app::App;
use crate::ui::types::Popup;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap, Table, Row, Cell};
use ratatui::Frame;

use super::utils::centered_rect;

pub fn draw_popup(f: &mut Frame, _app: &App, p: &Popup) {
    match p {
        Popup::Inspect { name, json, tab, .. } => {
            let area = centered_rect(90, 90, f.area());
            f.render_widget(Clear, area);
            
            let tabs = [" [1] Summary ", " [2] Config ", " [3] Network "];
            let tab_spans: Vec<Span> = tabs.iter().enumerate().map(|(i, &t)| {
                if i == *tab {
                    Span::styled(t, Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD))
                } else {
                    Span::raw(t)
                }
            }).collect();
            let tab_line = Line::from(tab_spans);

            let content = match tab {
                0 => {
                    let id = json["Id"].as_str().unwrap_or("-");
                    let created = json["Created"].as_str().unwrap_or("-");
                    let path = json["Path"].as_str().unwrap_or("-");
                    let args = json["Args"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(" ")).unwrap_or_default();
                    format!("ID: {id}\nName: {name}\nCreated: {created}\nPath: {path} {args}\n\n[Tab/Arrows] Switch tabs, [Esc] Close")
                }
                1 => {
                    let image = json["Config"]["Image"].as_str().unwrap_or("-");
                    let env = json["Config"]["Env"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
                    let labels = json["Config"]["Labels"].as_object().map(|o| o.iter().map(|(k,v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n")).unwrap_or_default();
                    format!("Image: {image}\n\n-- ENV --\n{env}\n\n-- Labels --\n{labels}")
                }
                2 => {
                    let nw = json["NetworkSettings"]["Networks"].as_object().map(|o| {
                        o.iter().map(|(k,v)| format!("{k}:\n  IP: {}\n  Gateway: {}", v["IPAddress"], v["Gateway"])).collect::<Vec<_>>().join("\n")
                    }).unwrap_or_else(|| "No network info".to_string());
                    format!("-- Networks --\n{nw}")
                }
                _ => "Unknown tab".to_string(),
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .title(tab_line);
            
            let p = Paragraph::new(content)
                .block(block)
                .wrap(Wrap { trim: false });
            f.render_widget(p, area);
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
        Popup::ConfirmBulkRemove { ids } => {
            let area = centered_rect(60, 25, f.area());
            f.render_widget(Clear, area);
            let msg = format!(
                "REMOVE {} containers?\nThis will STOP and DELETE the selected containers.\n\n[y/Enter]=Remove, [n/Esc]=Cancel",
                ids.len()
            );
            let w = Paragraph::new(msg)
                .block(Block::default().borders(Borders::ALL).title(" ⚠️  BULK REMOVE "))
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
            let area = centered_rect(65, 75, f.area());
            f.render_widget(Clear, area);
            let title = " 📊 Global System Health (Disk Usage) ";
            
            let chunks = ratatui::layout::Layout::default()
                .direction(ratatui::layout::Direction::Vertical)
                .constraints([
                    ratatui::layout::Constraint::Length(data.len() as u16 * 4 + 2),
                    ratatui::layout::Constraint::Min(0),
                    ratatui::layout::Constraint::Length(2),
                ])
                .margin(1)
                .split(area);

            f.render_widget(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan)).title(title), area);

            let mut current_y = chunks[0].y;
            for row in data {
                let row_area = ratatui::layout::Rect {
                    x: chunks[0].x + 1,
                    y: current_y,
                    width: chunks[0].width - 2,
                    height: 3,
                };
                current_y += 4;

                let row_chunks = ratatui::layout::Layout::default()
                    .direction(ratatui::layout::Direction::Vertical)
                    .constraints([
                        ratatui::layout::Constraint::Length(1),
                        ratatui::layout::Constraint::Length(1),
                        ratatui::layout::Constraint::Length(1),
                    ])
                    .split(row_area);

                f.render_widget(Paragraph::new(Line::from(vec![
                    Span::styled(format!("  {:<12}", row.kind), Style::default().fg(Color::Rgb(255, 170, 0)).add_modifier(Modifier::BOLD)),
                    Span::raw(format!(" Total: {}  Active: {}  Size: {}", row.total, row.active, row.size)),
                ])), row_chunks[0]);

                let gauge = ratatui::widgets::Gauge::default()
                    .block(Block::default())
                    .gauge_style(Style::default().fg(Color::Green).bg(Color::Rgb(40, 40, 40)))
                    .percent(row.reclaimable_percent as u16)
                    .label(format!("Reclaimable: {} ({:.1}%)", row.reclaimable, row.reclaimable_percent));
                f.render_widget(gauge, row_chunks[1]);
            }

            let help_text = Paragraph::new("   [X]:System Prune (Cleanup)   [Esc]:Close ")
                .style(Style::default().fg(Color::Gray));
            f.render_widget(help_text, chunks[2]);
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
        Popup::FileExplorer { name, path, files, selected, .. } => {
            let area = centered_rect(80, 80, f.area());
            f.render_widget(Clear, area);
            let title = format!(" 📂 Explorer: {name} [{path}] ");

            let rows = files.iter().enumerate().map(|(i, (fname, is_dir))| {
                let style = if i == *selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let icon = if *is_dir { "📁" } else { "📄" };
                Row::new(vec![Cell::from(icon), Cell::from(fname.clone())]).style(style)
            });

            let t = Table::new(
                rows,
                [
                    ratatui::layout::Constraint::Length(4),
                    ratatui::layout::Constraint::Min(40),
                ],
            )
            .block(Block::default().borders(Borders::ALL).title(title));

            f.render_widget(t, area);

            let help_area = ratatui::layout::Rect {
                x: area.x,
                y: area.y + area.height,
                width: area.width,
                height: 1,
            };
            let help_text = Paragraph::new(" ↑/↓:Nav  Enter/→:Open/Enter  Backspace/←:Back  Esc:Close ").style(Style::default().fg(Color::Gray));
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
